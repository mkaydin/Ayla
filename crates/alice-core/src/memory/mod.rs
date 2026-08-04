// ── Memory Manager ─────────────────────────────────────────
// Orchestrates all memory layers. Degrades gracefully when DB is offline.

/// Truncate a string to at most `max_bytes` without splitting a multi-byte UTF-8
/// character. Fixes audit issue R35 — the old `&text[..text.len().min(N)]` pattern
/// panics when byte N falls inside a character like `─` or emoji.
pub fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub mod config {
    pub use crate::config::*;
}
pub mod consolidator;
pub mod context;
pub mod conversation;
pub mod document_store;
pub mod embeddings;
pub mod episodic;
pub mod extraction;
pub mod graph;
pub mod graph_cleanup;
pub mod ingestion;
pub mod pipeline;
pub mod retrieval;
pub mod semantic;
pub mod summarizer;
pub mod working;

use crate::tools::types::ToolDispatch;
use crate::tools::ToolRegistry;
use config::MemoryConfig;
use consolidator::{ConsolidationReport, Consolidator};
use context::ContextBuilder;
use conversation::ConversationStore;
use document_store::DocumentStore;
use embeddings::Embedder;
use episodic::EpisodeStore;
use extraction::Extractor;
use graph::KnowledgeGraph;
use ingestion::IngestionPipeline;
use pipeline::MemoryPipeline;
use retrieval::HybridRetrieval;
use semantic::SemanticStore;
use std::collections::HashMap;
use std::sync::Arc;
use summarizer::Summarizer;
use tokio::sync::Mutex;
use working::WorkingMemory;

pub struct MemoryManager {
    pub config: MemoryConfig,
    pub tool_registry: ToolRegistry,
    pub graph: Arc<Mutex<KnowledgeGraph>>,
    text_store: Arc<Mutex<HashMap<String, String>>>,
    pub document_store: Option<Arc<DocumentStore>>,
    db_ready: bool,
    pipeline: Option<Arc<Mutex<MemoryPipeline>>>,
    ingestion: Option<Arc<Mutex<IngestionPipeline>>>,
    retrieval: Option<HybridRetrieval>,
    consolidator: Option<Arc<Mutex<Consolidator>>>,
}

impl MemoryManager {
    pub async fn new(config: MemoryConfig) -> Self {
        let tool_registry = ToolRegistry::new();
        let text_store = Arc::new(Mutex::new(HashMap::new()));

        let mut last_error = String::new();
        let mut stores = None;
        for attempt in 1..=3 {
            match Self::init_stores(&config, text_store.clone()).await {
                Ok(initialized) => {
                    stores = Some(initialized);
                    break;
                }
                Err(error) => {
                    last_error = error;
                    if attempt < 3 {
                        let delay = std::time::Duration::from_secs(1 << (attempt - 1));
                        log::warn!(
                            "Memory persistence init attempt {}/3 failed: {}. Retrying in {}s",
                            attempt,
                            last_error,
                            delay.as_secs()
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        match stores {
            Some((graph, pipeline, ingestion, mut retrieval, consolidator)) => {
                log::info!("Memory system initialized with persistent storage");
                let document_store = Self::init_document_store(&config).await.ok();
                if let Some(ref ds) = document_store {
                    retrieval = retrieval.with_document_store(ds.clone());
                }
                let manager = Self {
                    config,
                    tool_registry,
                    graph,
                    text_store,
                    document_store,
                    db_ready: true,
                    pipeline: Some(pipeline),
                    ingestion: Some(ingestion),
                    retrieval: Some(retrieval),
                    consolidator: Some(consolidator),
                };
                manager.spawn_consolidation_loop();
                manager
            }
            None => {
                log::warn!(
                    "Memory system started without persistence after 3 attempts: {}",
                    last_error
                );
                Self::offline_with(config, tool_registry, text_store)
            }
        }
    }

    /// Construct an immediately available degraded backend without contacting
    /// external services. Used by protocol tests and diagnostic tooling.
    pub fn offline(config: MemoryConfig) -> Self {
        Self::offline_with(
            config,
            ToolRegistry::new(),
            Arc::new(Mutex::new(HashMap::new())),
        )
    }

    fn offline_with(
        config: MemoryConfig,
        tool_registry: ToolRegistry,
        text_store: Arc<Mutex<HashMap<String, String>>>,
    ) -> Self {
        Self {
            config,
            tool_registry,
            graph: Arc::new(Mutex::new(KnowledgeGraph::new())),
            text_store,
            document_store: None,
            db_ready: false,
            pipeline: None,
            ingestion: None,
            retrieval: None,
            consolidator: None,
        }
    }

    pub fn is_db_ready(&self) -> bool {
        self.db_ready
    }

    pub async fn get_document_text(&self, name: &str) -> String {
        self.text_store
            .lock()
            .await
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    /// Transitional adapter access while the remaining host commands move to
    /// `BackendApi`. Returning a cloned `Arc` keeps ownership in the core.
    pub fn ingestion_pipeline(&self) -> Option<Arc<Mutex<IngestionPipeline>>> {
        self.ingestion.clone()
    }

    pub async fn store_document_text(&self, name: String, text: String) -> usize {
        let mut store = self.text_store.lock().await;
        store.insert(name, text);
        store.len()
    }

    pub async fn remove_document_text(&self, name: &str) {
        self.text_store.lock().await.remove(name);
    }

    async fn init_stores(
        config: &MemoryConfig,
        text_store: Arc<Mutex<HashMap<String, String>>>,
    ) -> Result<
        (
            Arc<Mutex<KnowledgeGraph>>,
            Arc<Mutex<MemoryPipeline>>,
            Arc<Mutex<IngestionPipeline>>,
            HybridRetrieval,
            Arc<Mutex<Consolidator>>,
        ),
        String,
    > {
        use crate::db::qdrant::QdrantDB;
        use crate::db::surreal::SurrealDB;

        let surreal = SurrealDB::connect(
            &config.surrealdb_url,
            &config.surrealdb_ns,
            &config.surrealdb_db,
            &config.surrealdb_user,
            &config.surrealdb_pass,
        )
        .await
        .map_err(|e| format!("SurrealDB: {}", e))?;
        let surreal = Arc::new(surreal);

        let qdrant = QdrantDB::connect(&config.qdrant_url)
            .await
            .map_err(|e| format!("Qdrant: {}", e))?;
        let qdrant = Arc::new(qdrant);

        // Load persisted graph
        let mut graph = KnowledgeGraph::with_db(surreal.clone());
        graph.load().await?;

        // Load persisted text store
        surreal.ensure_table("text_store").await?;
        if let Ok(rows) = surreal.query("SELECT * FROM text_store;").await {
            for row in rows {
                if let (Some(k), Some(v)) = (
                    row.get("key").and_then(|v| v.as_str()),
                    row.get("value").and_then(|v| v.as_str()),
                ) {
                    text_store.lock().await.insert(k.to_string(), v.to_string());
                }
            }
            log::info!(
                "Text store loaded: {} documents",
                text_store.lock().await.len()
            );
        }

        let conversation = ConversationStore::new(surreal.clone());
        conversation.init_table().await?;

        let semantic_store = SemanticStore::new(surreal.clone());
        semantic_store.init_table().await?;

        let embedder = Embedder::new(config.clone());
        let episode = EpisodeStore::new(
            surreal.clone(),
            qdrant.clone(),
            embedder.clone(),
            config.embed_dim as u64,
        );
        episode.init_table().await?;

        let summarizer = Summarizer::new(config.clone());
        let extractor = Extractor::new(config.clone());
        let shared_graph = Arc::new(Mutex::new(graph));

        // Pipeline uses the SHARED persisted graph (was: KnowledgeGraph::new(), which silently
        // discarded every extracted entity/relation on restart — issue R1).
        let pipeline = MemoryPipeline::new(
            WorkingMemory::new(&uuid::Uuid::new_v4().to_string()),
            ConversationStore::new(surreal.clone()),
            EpisodeStore::new(
                surreal.clone(),
                qdrant.clone(),
                embedder.clone(),
                config.embed_dim as u64,
            ),
            SemanticStore::new(surreal.clone()),
            shared_graph.clone(),
            summarizer.clone(),
            extractor.clone(),
        );
        let pipeline = Arc::new(Mutex::new(pipeline));

        // Ingestion uses the SHARED persisted graph
        let ingestion = IngestionPipeline::new(
            ConversationStore::new(surreal.clone()),
            episode,
            semantic_store.clone(),
            shared_graph.clone(),
            extractor.clone(),
            summarizer.clone(),
        );
        let ingestion = Arc::new(Mutex::new(ingestion));

        // Retrieval uses the SHARED persisted graph (was: KnowledgeGraph::new() → empty
        // related_entities, issue R2).
        let retrieval = HybridRetrieval::new(
            shared_graph.clone(),
            semantic_store.clone(),
            EpisodeStore::new(
                surreal.clone(),
                qdrant.clone(),
                embedder.clone(),
                config.embed_dim as u64,
            ),
            config.clone(),
        );

        let consolidator = Arc::new(Mutex::new(Consolidator::new(
            surreal.clone(),
            semantic_store.clone(),
            shared_graph.clone(),
        )));

        Ok((shared_graph, pipeline, ingestion, retrieval, consolidator))
    }

    async fn init_document_store(config: &MemoryConfig) -> Result<Arc<DocumentStore>, String> {
        use crate::db::qdrant::QdrantDB;
        let qdrant = QdrantDB::connect(&config.qdrant_url).await?;
        let qdrant = Arc::new(qdrant);
        let embedder = Embedder::new(config.clone());
        let store = DocumentStore::new(qdrant, embedder, config.embed_dim as u64);
        store.init().await?;
        log::info!("Document store initialized (Qdrant 'documents' collection, 1024-dim)");
        Ok(Arc::new(store))
    }

    fn spawn_consolidation_loop(&self) {
        if let Some(ref c) = self.consolidator {
            let consolidator = c.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1800)).await;
                    let mut c = consolidator.lock().await;
                    match c.run().await {
                        Ok(report) => {
                            log::info!(
                                "Consolidation: {} merged, {} pruned, {} reinforced",
                                report.merges,
                                report.pruned,
                                report.reinforced
                            );
                        }
                        Err(e) => log::warn!("Consolidation failed: {}", e),
                    }
                }
            });
        }
    }

    fn db_err() -> String {
        "Database offline. Start Docker services: docker compose up -d".into()
    }

    pub async fn enrich_context(&self, query: &str, base_prompt: &str) -> Result<String, String> {
        let mut context_parts: Vec<String> = vec![];

        // Search in-memory text store (works even when DB is offline)
        {
            let store = self.text_store.lock().await;
            if !store.is_empty() {
                let query_lower = query.to_lowercase();
                let words: Vec<&str> = query_lower.split_whitespace().collect();
                let mut matches: Vec<(&String, &String)> = store
                    .iter()
                    .filter(|(_, text)| {
                        let tl = text.to_lowercase();
                        words.iter().any(|w| tl.contains(w))
                    })
                    .collect();
                matches.sort_by_key(|(_, t)| -(t.len() as i64));
                matches.truncate(3);

                if !matches.is_empty() {
                    context_parts.push("## Ingested Documents".into());
                    for (name, text) in matches {
                        let truncated = safe_truncate(text, 2000);
                        context_parts.push(format!("### {}\n{}\n", name, truncated));
                    }
                }
            }
        }

        // Try DB-backed retrieval
        let ctx = match &self.retrieval {
            Some(r) => r.retrieve(query).await.unwrap_or_default(),
            None => retrieval::RetrievedContext::default(),
        };

        let db_context = ContextBuilder::build_memory_context(&ctx);
        if !db_context.is_empty() {
            context_parts.push(db_context);
        }

        let memory_prefix = context_parts.join("\n");
        Ok(format!("{}{}", memory_prefix, base_prompt))
    }

    pub async fn process_turn(&self, user_msg: &str, assistant_msg: &str) -> Result<(), String> {
        match &self.pipeline {
            Some(p) => {
                let mut p = p.lock().await;
                p.process_turn(user_msg, assistant_msg).await
            }
            None => Err(Self::db_err()),
        }
    }

    pub async fn process_with_tools(
        &self,
        user_message: &str,
        base_prompt: &str,
    ) -> Result<(String, Option<String>), String> {
        let dispatch = self.try_dispatch_tool(user_message).await;
        if let Some((tool_name, query, file_path)) = dispatch {
            match self
                .tool_registry
                .execute(&tool_name, &query, file_path.as_deref())
                .await
            {
                Ok(result) => {
                    let status = result.summary.clone();
                    if let Some(ref ing) = self.ingestion {
                        let ingestion = ing.clone();
                        tokio::spawn(async move {
                            let mut i = ingestion.lock().await;
                            let _ = i.ingest(result, None).await;
                        });
                    }
                    return Ok((base_prompt.to_string(), Some(status)));
                }
                Err(e) => return Ok((base_prompt.to_string(), Some(format!("Tool error: {}", e)))),
            }
        }
        let enriched = self.enrich_context(user_message, base_prompt).await?;
        Ok((enriched, None))
    }

    async fn try_dispatch_tool(
        &self,
        user_message: &str,
    ) -> Option<(String, String, Option<String>)> {
        let prompt = ToolRegistry::dispatch_prompt(user_message);
        let num_ctx = self.config.ollama_context_for(prompt.len(), 128);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;
        let body = serde_json::json!({
            "model": self.config.llm_model,
            "messages": [{ "role": "user", "content": prompt }],
            "stream": false,
            "options": {
                "temperature": 0.0,
                "num_predict": 128,
                "num_ctx": num_ctx,
            },
        });
        let resp = client
            .post(format!("{}/api/chat", self.config.ollama_url))
            .json(&body)
            .send()
            .await
            .ok()?;
        let json: serde_json::Value = resp.json().await.ok()?;
        let content = json["message"]["content"].as_str()?;
        let cleaned = content.trim();
        let json_str = if let (Some(s), Some(e)) = (cleaned.find('{'), cleaned.rfind('}')) {
            &cleaned[s..=e]
        } else {
            return None;
        };
        let dispatch: ToolDispatch = serde_json::from_str(json_str).ok()?;
        if dispatch.tool == "none" {
            return None;
        }
        Some((dispatch.tool, dispatch.query, dispatch.file_path))
    }

    pub async fn run_consolidation(&self) -> Result<ConsolidationReport, String> {
        match &self.consolidator {
            Some(c) => {
                let mut c = c.lock().await;
                c.run().await
            }
            None => Err(Self::db_err()),
        }
    }
}

pub async fn save_text_store(manager: &MemoryManager) {
    let store = manager.text_store.lock().await;
    if store.is_empty() || !manager.db_ready {
        return;
    }
    use crate::db::surreal::SurrealDB;
    if let Ok(db) = SurrealDB::connect(
        &manager.config.surrealdb_url,
        &manager.config.surrealdb_ns,
        &manager.config.surrealdb_db,
        &manager.config.surrealdb_user,
        &manager.config.surrealdb_pass,
    )
    .await
    {
        let _ = db.ensure_table("text_store").await;
        let _ = db.query("DELETE text_store;").await;
        for (key, value) in store.iter() {
            let _ = db
                .insert(
                    "text_store",
                    serde_json::json!({"key": key, "value": value}),
                )
                .await;
        }
        log::info!("Text store saved: {} docs", store.len());
    }
}
