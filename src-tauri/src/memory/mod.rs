// ── Memory Manager ─────────────────────────────────────────
// Orchestrates all memory layers. Degrades gracefully when DB is offline.

pub mod config;
pub mod commands;
pub mod working;
pub mod conversation;
pub mod episodic;
pub mod semantic;
pub mod graph;
pub mod embeddings;
pub mod extraction;
pub mod summarizer;
pub mod consolidator;
pub mod retrieval;
pub mod context;
pub mod pipeline;
pub mod ingestion;

use config::MemoryConfig;
use working::WorkingMemory;
use conversation::ConversationStore;
use episodic::EpisodeStore;
use semantic::SemanticStore;
use graph::KnowledgeGraph;
use embeddings::Embedder;
use extraction::Extractor;
use summarizer::Summarizer;
use consolidator::{Consolidator, ConsolidationReport};
use retrieval::HybridRetrieval;
use context::ContextBuilder;
use pipeline::MemoryPipeline;
use ingestion::IngestionPipeline;
use crate::tools::{ToolRegistry, IngestTool};
use crate::tools::types::ToolDispatch;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;

pub struct MemoryManager {
    pub config: MemoryConfig,
    pub tool_registry: ToolRegistry,
    pub graph: Arc<Mutex<KnowledgeGraph>>,
    text_store: Arc<Mutex<HashMap<String, String>>>,
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

        match Self::init_stores(&config, text_store.clone()).await {
            Ok((graph, pipeline, ingestion, retrieval, consolidator)) => {
                log::info!("Memory system initialized with persistent storage");
                let manager = Self {
                    config,
                    tool_registry,
                    graph,
                    text_store,
                    db_ready: true,
                    pipeline: Some(pipeline),
                    ingestion: Some(ingestion),
                    retrieval: Some(retrieval),
                    consolidator: Some(consolidator),
                };
                manager.spawn_consolidation_loop();
                manager
            }
            Err(e) => {
                log::warn!("Memory system started without persistence: {}", e);
                Self {
                    config,
                    tool_registry,
                    graph: Arc::new(Mutex::new(KnowledgeGraph::new())),
                    text_store,
                    db_ready: false,
                    pipeline: None,
                    ingestion: None,
                    retrieval: None,
                    consolidator: None,
                }
            }
        }
    }

    async fn init_stores(
        config: &MemoryConfig,
        text_store: Arc<Mutex<HashMap<String, String>>>,
    ) -> Result<(
        Arc<Mutex<KnowledgeGraph>>,
        Arc<Mutex<MemoryPipeline>>,
        Arc<Mutex<IngestionPipeline>>,
        HybridRetrieval,
        Arc<Mutex<Consolidator>>,
    ), String> {
        use crate::db::surreal::SurrealDB;
        use crate::db::qdrant::QdrantDB;

        let surreal = SurrealDB::connect(
            &config.surrealdb_url, &config.surrealdb_ns, &config.surrealdb_db,
            "root", "root",
        ).await.map_err(|e| format!("SurrealDB: {}", e))?;
        let surreal = Arc::new(surreal);

        let qdrant = QdrantDB::connect(&config.qdrant_url).await.map_err(|e| format!("Qdrant: {}", e))?;
        let qdrant = Arc::new(qdrant);

        // Load persisted graph
        let mut graph = KnowledgeGraph::with_db(surreal.clone());
        graph.load().await?;

        // Load persisted text store
        surreal.ensure_table("text_store").await?;
        if let Ok(rows) = surreal.query("SELECT * FROM text_store;").await {
            for row in rows {
                if let (Some(k), Some(v)) = (row.get("key").and_then(|v| v.as_str()), row.get("value").and_then(|v| v.as_str())) {
                    text_store.lock().await.insert(k.to_string(), v.to_string());
                }
            }
            log::info!("Text store loaded: {} documents", text_store.lock().await.len());
        }

        let conversation = ConversationStore::new(surreal.clone());
        conversation.init_table().await?;

        let semantic_store = SemanticStore::new(surreal.clone());
        semantic_store.init_table().await?;

        let embedder = Embedder::new(config.clone());
        let episode = EpisodeStore::new(surreal.clone(), qdrant.clone(), embedder.clone());
        episode.init_table().await?;

        let summarizer = Summarizer::new(config.clone());
        let extractor = Extractor::new(config.clone());
        let shared_graph = Arc::new(Mutex::new(graph));

        // Pipeline uses its own local graph for turn processing
        let pipeline = MemoryPipeline::new(
            WorkingMemory::new(&uuid::Uuid::new_v4().to_string()),
            ConversationStore::new(surreal.clone()),
            EpisodeStore::new(surreal.clone(), qdrant.clone(), embedder.clone()),
            SemanticStore::new(surreal.clone()),
            KnowledgeGraph::new(),
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

        // Retrieval
        let retrieval = HybridRetrieval::new(
            KnowledgeGraph::new(),
            semantic_store.clone(),
            EpisodeStore::new(surreal.clone(), qdrant.clone(), embedder.clone()),
            embedder,
            config.clone(),
        );

        let consolidator = Arc::new(Mutex::new(Consolidator::new(surreal.clone(), semantic_store.clone(), KnowledgeGraph::new())));

        Ok((shared_graph, pipeline, ingestion, retrieval, consolidator))
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
                            log::info!("Consolidation: {} merged, {} pruned, {} reinforced",
                                report.merges, report.pruned, report.reinforced);
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
                let mut matches: Vec<(&String, &String)> = store.iter()
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
                        let truncated = &text[..text.len().min(2000)];
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
            Some(p) => { let mut p = p.lock().await; p.process_turn(user_msg, assistant_msg).await }
            None => Err(Self::db_err()),
        }
    }

    pub async fn process_with_tools(
        &self, user_message: &str, base_prompt: &str,
    ) -> Result<(String, Option<String>), String> {
        let dispatch = self.try_dispatch_tool(user_message).await;
        if let Some((tool_name, query, file_path)) = dispatch {
            match self.tool_registry.execute(&tool_name, &query, file_path.as_deref()).await {
                Ok(result) => {
                    let status = result.summary.clone();
                    if let Some(ref ing) = self.ingestion {
                        let ingestion = ing.clone();
                        tokio::spawn(async move {
                            let mut i = ingestion.lock().await;
                            let _ = i.ingest(result).await;
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

    async fn try_dispatch_tool(&self, user_message: &str) -> Option<(String, String, Option<String>)> {
        let prompt = ToolRegistry::dispatch_prompt(user_message);
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": self.config.llm_model,
            "messages": [{ "role": "user", "content": prompt }],
            "stream": false,
            "options": { "temperature": 0.0, "num_predict": 128 },
        });
        let resp = client.post(&format!("{}/api/chat", self.config.ollama_url)).json(&body).send().await.ok()?;
        let json: serde_json::Value = resp.json().await.ok()?;
        let content = json["message"]["content"].as_str()?;
        let cleaned = content.trim();
        let json_str = if let (Some(s), Some(e)) = (cleaned.find('{'), cleaned.rfind('}')) {
            &cleaned[s..=e]
        } else { return None; };
        let dispatch: ToolDispatch = serde_json::from_str(json_str).ok()?;
        if dispatch.tool == "none" { return None; }
        Some((dispatch.tool, dispatch.query, dispatch.file_path))
    }

    pub async fn run_consolidation(&self) -> Result<ConsolidationReport, String> {
        match &self.consolidator {
            Some(c) => { let mut c = c.lock().await; c.run().await }
            None => Err(Self::db_err()),
        }
    }
}

pub async fn save_text_store(manager: &MemoryManager) {
    let store = manager.text_store.lock().await;
    if store.is_empty() || !manager.db_ready { return; }
    use crate::db::surreal::SurrealDB;
    if let Ok(db) = SurrealDB::connect(
        &manager.config.surrealdb_url, &manager.config.surrealdb_ns, &manager.config.surrealdb_db,
        "root", "root",
    ).await {
        let _ = db.ensure_table("text_store").await;
        let _ = db.query("DELETE text_store;").await;
        for (key, value) in store.iter() {
            let _ = db.insert("text_store", serde_json::json!({"key": key, "value": value})).await;
        }
        log::info!("Text store saved: {} docs", store.len());
    }
}
