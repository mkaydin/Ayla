static KNOWLEDGE_WRITER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use crate::db::surreal::SurrealDB;
use crate::memory::graph::NodeType;
use crate::memory::graph_cleanup::{CleanupEdge, CleanupNode, GraphCleanupAnalyzer};
use crate::memory::graph_v2::{
    GraphBackupPruneResult, GraphCommitReceipt, GraphImportBatch, GraphV2ApplicationData,
    GraphV2PageData, GraphValidationReport,
};
use crate::memory::MemoryManager;
use crate::processes::ProcessRegistry;
use crate::tools::types::{IngestionResult, SourceInfo};
use crate::{BackendError, EventSink, ResourceLocator, StreamSink};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const MAX_TEXT_PAYLOAD: usize = 50 * 1024 * 1024;
const MAX_MESSAGES_PAYLOAD: usize = 10 * 1024 * 1024;
const MAX_SOURCE_FILE: u64 = 100 * 1024 * 1024;
const CHAT_MAX_OUTPUT_TOKENS: usize = 4_096;
const MAX_CODE_SNIPPET_BYTES: usize = 2 * 1024 * 1024;

fn cleanup_node_type(value: &str) -> Option<NodeType> {
    match value {
        "Person" => Some(NodeType::Person),
        "Project" => Some(NodeType::Project),
        "Tool" => Some(NodeType::Tool),
        "Concept" => Some(NodeType::Concept),
        "Other" => Some(NodeType::Other),
        _ => None,
    }
}

fn resolve_graph_name(value: &str, resolutions: &HashMap<String, String>) -> String {
    let mut current = value.trim().to_lowercase();
    let mut visited = HashSet::new();
    while visited.insert(current.clone()) {
        let Some(next) = resolutions.get(&current) else {
            break;
        };
        current = next.clone();
    }
    current
}

///
/// Operations are migrated here incrementally. Adapters must preserve their
/// existing public command names and JSON shapes while delegating to this API.
pub struct BackendApi<'manager> {
    manager: &'manager MemoryManager,
}

struct CodeAnalysisWorkerGuard(ayla_code_analysis::AnalysisControl);

impl Drop for CodeAnalysisWorkerGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct CodeAnalysisAsyncWorkerGuard(tokio::task::AbortHandle);

impl Drop for CodeAnalysisAsyncWorkerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeBaseData {
    pub nodes: Vec<KnowledgeNode>,
    pub edges: Vec<KnowledgeEdge>,
    pub files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeNode {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub source: String,
    pub sources: Vec<String>,
    pub aliases: Vec<String>,
    pub meaning: String,
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeEdge {
    pub from: String,
    pub relation: String,
    pub to: String,
    pub confidence: f32,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileRecord {
    #[serde(default)]
    pub source_id: String,
    #[serde(default)]
    pub status: String,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub source: String,
    pub date: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatSessionRecord {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMessageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessageRecord {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHistoryRecord {
    pub status: Value,
    pub warnings: Value,
    pub model: Value,
    pub source_outcomes: Value,
    pub id: String,
    pub search_id: String,
    pub query: Value,
    pub mode: Value,
    pub timestamp: Value,
    pub updated_at: Value,
    pub knowledge_document: Value,
    pub knowledge_ingested: bool,
    pub synthesis: Value,
    pub papers: Value,
    pub documents: Vec<Value>,
    pub total_results_found: Value,
    pub pages_rendered: Value,
    pub persisted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentSearchResult {
    pub text: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessMessageResult {
    pub system_prompt: String,
    pub tool_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsolidationResult {
    pub merges: usize,
    pub pruned: usize,
    pub reinforced: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphCleanupGroup {
    pub canonical: String,
    pub aliases: Vec<String>,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNodeRefinement {
    pub previous_name: String,
    pub canonical_name: String,
    pub meaning: String,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphRelationRefinement {
    pub from: String,
    pub to: String,
    pub previous_relation: String,
    pub improved_relation: String,
    pub confidence: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphCleanupResult {
    pub proposed_groups: usize,
    pub proposed_node_refinements: usize,
    pub proposed_relation_refinements: usize,
    pub merged_groups: usize,
    pub merged_nodes: usize,
    pub renamed_nodes: usize,
    pub updated_meanings: usize,
    pub updated_node_types: usize,
    pub relabeled_relations: usize,
    pub deduplicated_edges: usize,
    pub skipped_groups: usize,
    pub groups: Vec<GraphCleanupGroup>,
    pub node_refinements: Vec<GraphNodeRefinement>,
    pub relation_refinements: Vec<GraphRelationRefinement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuickExtractResult {
    pub text: String,
    pub filename: String,
    pub chars: usize,
    pub chunks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrResult {
    pub text: String,
    pub pages: String,
    pub chars: usize,
    pub pid: String,
    pub source_name: String,
    pub pages_processed: usize,
    pub pages_skipped: usize,
    pub total_pages: usize,
    pub page_results: Vec<ayla_ocr::OcrPageResult>,
    pub models: ayla_ocr::OcrModels,
    pub warnings: Vec<ayla_ocr::OcrWarning>,
    pub knowledge_mode: ayla_ocr::OcrKnowledgeMode,
    pub performance: ayla_ocr::OcrPerformanceMetrics,
    pub cache_key: Option<String>,
    pub markdown_artifact: crate::ArtifactRef,
    pub layout_artifact: crate::ArtifactRef,
    pub knowledge_base_artifact: crate::ArtifactRef,
    pub structured_artifact: crate::ArtifactRef,
    pub knowledge_entity_count: usize,
    pub knowledge_relation_count: usize,
    pub knowledge_validation: GraphValidationReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrKnowledgeIngestion {
    pub operation_id: String,
    pub source_name: String,
    pub chars: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrGraphMutation {
    pub operation_id: String,
    pub source_id: String,
    pub receipt: GraphCommitReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnalysisRequest {
    pub source_type: String,
    pub directory_path: Option<String>,
    pub code: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub scope: Option<String>,
    #[serde(default)]
    pub semantic_mode: Option<String>,
    #[serde(default)]
    pub parent_operation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnalysisResult {
    pub pid: String,
    pub project_name: String,
    pub repository_uid: String,
    pub entity_count: usize,
    pub relation_count: usize,
    pub diagnostics: Vec<ayla_code_analysis::Diagnostic>,
    pub validation: GraphValidationReport,
    pub artifacts: crate::CodeAnalysisArtifacts,
    pub parent_operation_id: Option<String>,
    pub created_at: String,
    pub semantic_mode: String,
    pub diff: Option<crate::artifacts::CodeAnalysisDiffSummary>,
    pub graph_approved: bool,
    pub vectors_indexed: bool,
    pub source_snapshot_stored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnalysisApproval {
    pub operation_id: String,
    pub source_id: String,
    pub receipt: GraphCommitReceipt,
    pub entity_vectors: usize,
    pub document_vectors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnalysisRemoval {
    pub operation_id: String,
    pub source_id: String,
    pub receipt: GraphCommitReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnalysisRunDeletion {
    pub operation_id: String,
    pub graph_removed: bool,
    pub vectors_removed: bool,
    pub artifacts_removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphV2Rollback {
    pub target_backup_id: String,
    pub receipt: GraphCommitReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyGraphV2Migration {
    pub source_id: String,
    pub migrated_nodes: usize,
    pub migrated_edges: usize,
    pub receipt: GraphCommitReceipt,
}

impl<'manager> BackendApi<'manager> {
    pub fn new(manager: &'manager MemoryManager) -> Self {
        Self { manager }
    }

    pub fn check_db_ready(&self) -> bool {
        self.manager.is_db_ready()
    }

    pub async fn get_kb_data(&self) -> KnowledgeBaseData {
        let graph = self.manager.graph.lock().await;
        let nodes = graph
            .all_nodes()
            .into_iter()
            .map(|node| KnowledgeNode {
                name: node.name.clone(),
                node_type: format!("{:?}", node.node_type),
                source: node.sources.first().cloned().unwrap_or_default(),
                sources: node.sources.clone(),
                aliases: node.aliases.clone(),
                meaning: node.meaning.clone(),
                created: node.created.clone(),
            })
            .collect();
        let edges = graph
            .all_edges()
            .into_iter()
            .map(|(from, relation, to, confidence, sources)| KnowledgeEdge {
                from,
                relation,
                to,
                confidence,
                sources,
            })
            .collect();
        KnowledgeBaseData {
            nodes,
            edges,
            // The UI retrieves file records through `get_files`; preserve the
            // historical `get_kb_data` response shape with an empty list here.
            files: Vec::new(),
        }
    }

    pub async fn get_graph_v2_data(&self) -> Result<GraphV2ApplicationData, BackendError> {
        let Some(store) = self.manager.graph_v2.as_ref() else {
            return Ok(GraphV2ApplicationData::unavailable());
        };
        store
            .lock()
            .await
            .application_data()
            .await
            .map_err(|error| BackendError::new("graph_v2_read", error))
    }

    pub async fn get_graph_v2_page(
        &self,
        kind: &str,
        offset: usize,
        limit: usize,
    ) -> Result<GraphV2PageData, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        store
            .lock()
            .await
            .application_page(kind, offset, limit)
            .await
            .map_err(|error| BackendError::new("graph_v2_read", error))
    }

    pub async fn migrate_legacy_graph_v2(&self) -> Result<LegacyGraphV2Migration, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let (nodes, edges) = {
            let graph = self.manager.graph.lock().await;
            (
                graph.all_nodes().into_iter().cloned().collect::<Vec<_>>(),
                graph.all_edges(),
            )
        };
        let source_id = "legacy-mirror:v1".to_string();
        let batch = GraphImportBatch::from_legacy(source_id.clone(), &nodes, &edges);
        let migrated_nodes = batch.nodes.len();
        let migrated_edges = batch.edges.len();
        let receipt = store
            .lock()
            .await
            .replace_legacy(batch)
            .await
            .map_err(|error| BackendError::new("graph_v2_migration", error))?;
        Ok(LegacyGraphV2Migration {
            source_id,
            migrated_nodes,
            migrated_edges,
            receipt,
        })
    }

    pub async fn prune_graph_v2_backups(
        &self,
        retention: usize,
    ) -> Result<GraphBackupPruneResult, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        store
            .lock()
            .await
            .prune_backups(retention)
            .await
            .map_err(|error| BackendError::new("graph_v2_maintenance", error))
    }

    pub async fn rollback_graph_v2(
        &self,
        backup_id: &str,
    ) -> Result<GraphV2Rollback, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let receipt = store
            .lock()
            .await
            .rollback(backup_id)
            .await
            .map_err(|error| BackendError::new("graph_v2_rollback", error))?;
        self.manager
            .graph
            .lock()
            .await
            .load()
            .await
            .map_err(BackendError::from)?;
        Ok(GraphV2Rollback {
            target_backup_id: backup_id.into(),
            receipt,
        })
    }

    pub async fn get_files(&self) -> Result<Vec<FileRecord>, BackendError> {
        log::info!("Files: querying kb_file table");
        if !self.manager.is_db_ready() {
            return Ok(Vec::new());
        }

        let config = &self.manager.config;
        let db = SurrealDB::connect(
            &config.surrealdb_url,
            &config.surrealdb_ns,
            &config.surrealdb_db,
            &config.surrealdb_user,
            &config.surrealdb_pass,
        )
        .await
        .map_err(BackendError::from)?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        let mut rows = db
            .query("SELECT name, path, chars, source, source_id, status, date FROM kb_file ORDER BY date DESC;")
            .await
            .map_err(BackendError::from)?;
        let operations = db
            .query("SELECT name, path, chars, source_type, status, created_at AS date FROM knowledge_operation WHERE status IN ['failed', 'approving'];")
            .await
            .map_err(BackendError::from)?;
        for mut operation in operations {
            let name = string_field(&operation, "name");
            operation["source"] = operation["source_type"].clone();
            if let Some(index) = rows
                .iter()
                .position(|record| string_field(record, "name") == name)
            {
                rows[index] = operation;
            } else {
                rows.push(operation);
            }
        }
        log::info!("Files: found {} records", rows.len());

        Ok(rows
            .iter()
            .map(|record| FileRecord {
                source_id: string_field(record, "source_id"),
                status: string_field(record, "status"),
                name: string_field(record, "name"),
                path: string_field(record, "path"),
                size: record
                    .get("chars")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                source: string_field(record, "source"),
                date: string_field(record, "date"),
                text: string_field(record, "text"),
            })
            .collect())
    }

    pub async fn get_document_text(&self, name: &str) -> String {
        let text = self.manager.get_document_text(name).await;
        if !text.is_empty() {
            return text;
        }
        // Explicit preview/retry reads one document; the file list never transfers
        // an unbounded collection of full document bodies across IPC.
        if let Ok(db) = self.connect_search_database().await {
            let vars = serde_json::json!({"name": name})
                .as_object()
                .unwrap()
                .clone();
            for sql in ["SELECT text FROM knowledge_operation WHERE name = $name AND status = 'failed' LIMIT 1;", "SELECT text FROM kb_file WHERE name = $name LIMIT 1;"] {
                if let Ok(rows) = db.query_with_vars(sql, vars.clone()).await {
                    if let Some(text) = rows.first().and_then(|row| row.get("text")).and_then(Value::as_str) { return text.to_string(); }
                }
            }
        }
        String::new()
    }

    pub async fn enrich_prompt(
        &self,
        user_query: &str,
        base_prompt: &str,
    ) -> Result<String, BackendError> {
        self.manager
            .enrich_context(user_query, base_prompt)
            .await
            .map_err(BackendError::from)
    }

    pub async fn process_turn(
        &self,
        user_message: &str,
        assistant_message: &str,
    ) -> Result<(), BackendError> {
        self.manager
            .process_turn(user_message, assistant_message)
            .await
            .map_err(BackendError::from)
    }

    pub async fn process_message(
        &self,
        query: &str,
        base_prompt: &str,
    ) -> Result<ProcessMessageResult, BackendError> {
        let (system_prompt, tool_status) = self
            .manager
            .process_with_tools(query, base_prompt)
            .await
            .map_err(BackendError::from)?;
        Ok(ProcessMessageResult {
            system_prompt,
            tool_status,
        })
    }

    pub async fn stream_chat(
        &self,
        messages: Vec<Value>,
        stream: &dyn StreamSink,
    ) -> Result<(), BackendError> {
        let payload_size = serde_json::to_vec(&messages)
            .map_err(|error| {
                BackendError::new("invalid_payload", format!("Chat payload: {error}"))
            })?
            .len();
        if payload_size > MAX_MESSAGES_PAYLOAD {
            return Err(BackendError::new(
                "payload_too_large",
                format!(
                    "Chat payload too large ({} bytes, max {})",
                    payload_size, MAX_MESSAGES_PAYLOAD
                ),
            ));
        }
        let num_ctx = self
            .manager
            .config
            .ollama_context_for(payload_size, CHAT_MAX_OUTPUT_TOKENS);
        log::debug!(
            "Ollama chat: payload={} bytes, num_ctx={num_ctx}",
            payload_size
        );

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| BackendError::new("chat_client", format!("Chat client: {error}")))?;
        let mut response = client
            .post(format!("{}/api/chat", self.manager.config.ollama_url))
            .json(&serde_json::json!({
                "model": self.manager.config.llm_model,
                "messages": messages,
                "stream": true,
                "options": {
                    "temperature": 0.7,
                    "num_ctx": num_ctx,
                    "num_predict": CHAT_MAX_OUTPUT_TOKENS,
                },
            }))
            .send()
            .await
            .map_err(|error| BackendError::new("ollama_request", format!("Ollama: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::new(
                "ollama_http",
                format!(
                    "Ollama HTTP {}: {}",
                    status,
                    crate::memory::safe_truncate(&body, 300)
                ),
            ));
        }

        let mut pending = Vec::<u8>::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            BackendError::new("ollama_stream", format!("Ollama stream: {error}"))
        })? {
            pending.extend_from_slice(&chunk);
            while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=newline).collect();
                let line = line.strip_suffix(b"\n").unwrap_or(&line);
                if line.is_empty() {
                    continue;
                }
                emit_chat_delta(line, stream)?;
            }
        }
        if !pending.is_empty() {
            // Ollama normally terminates records with a newline. Preserve the
            // previous tolerant behavior for a final unterminated record.
            if let Ok(value) = serde_json::from_slice::<Value>(&pending) {
                emit_chat_delta_value(&value, stream)?;
            }
        }
        stream
            .send(serde_json::json!({ "type": "done" }))
            .map_err(chat_stream_error)?;
        Ok(())
    }

    pub async fn run_consolidation(&self) -> Result<ConsolidationResult, BackendError> {
        let report = self
            .manager
            .run_consolidation()
            .await
            .map_err(BackendError::from)?;
        Ok(ConsolidationResult {
            merges: report.merges,
            pruned: report.pruned,
            reinforced: report.reinforced,
        })
    }

    pub async fn ingest_text(
        &self,
        text: String,
        source: String,
        source_type: String,
        events: &dyn EventSink,
    ) -> Result<String, BackendError> {
        validate_ingest_payload_size(text.len())?;
        log::info!(
            "Ingest: {} chars from '{}' (type: {})",
            text.len(),
            source,
            source_type
        );
        let result = IngestionResult {
            summary: format!(
                "Ingested from {}: {}",
                source_type,
                crate::memory::safe_truncate(&source, 60)
            ),
            entities: Vec::new(),
            facts: Vec::new(),
            relations: Vec::new(),
            full_text: text,
            source: SourceInfo {
                tool: source_type.clone(),
                input: source.clone(),
                output_path: None,
                metadata: serde_json::json!({}),
            },
            timestamp: chrono::Utc::now(),
        };

        match self.manager.ingestion_pipeline() {
            Some(pipeline) => {
                log::info!("Ingest: running ingestion pipeline...");
                let document_text = result.full_text.clone();
                let mut ingestion = pipeline.lock().await;
                let message = ingestion
                    .ingest(result, Some(events))
                    .await
                    .map_err(BackendError::from)?;
                let db = self.connect_database().await?;
                db.ensure_table("text_store")
                    .await
                    .map_err(BackendError::from)?;
                let vars = serde_json::json!({"key": source, "value": document_text})
                    .as_object()
                    .unwrap()
                    .clone();
                db.exec_with_vars("BEGIN TRANSACTION; DELETE text_store WHERE key = $key; CREATE text_store CONTENT { key: $key, value: $value }; COMMIT TRANSACTION;", vars).await.map_err(BackendError::from)?;
                self.manager
                    .store_document_text(source, document_text)
                    .await;
                log::info!("Ingest: {}", message);
                Ok(message)
            }
            None => Err(BackendError::new("db_offline", "Persistent knowledge storage is unavailable. Start the databases and retry approval; no in-memory-only success is reported.")),
        }
    }

    pub async fn analyze_and_merge_kb(
        &self,
        events: &dyn EventSink,
    ) -> Result<GraphCleanupResult, BackendError> {
        let (nodes, edges) = {
            let graph = self.manager.graph.lock().await;
            let edges = graph
                .all_edges()
                .into_iter()
                .map(|(from, relation, to, confidence, sources)| CleanupEdge {
                    from,
                    relation,
                    to,
                    confidence,
                    sources: sources.into_iter().take(12).collect(),
                })
                .collect::<Vec<_>>();
            let mut contexts: HashMap<String, Vec<String>> = HashMap::new();
            for edge in &edges {
                contexts
                    .entry(edge.from.to_lowercase())
                    .or_default()
                    .push(format!("{} {} {}", edge.from, edge.relation, edge.to));
                contexts
                    .entry(edge.to.to_lowercase())
                    .or_default()
                    .push(format!("{} {} {}", edge.from, edge.relation, edge.to));
            }
            let nodes = graph
                .all_nodes()
                .into_iter()
                .map(|node| {
                    let mut context = contexts
                        .remove(&node.name.to_lowercase())
                        .unwrap_or_default();
                    context.sort();
                    context.dedup();
                    context.truncate(12);
                    CleanupNode {
                        name: node.name.clone(),
                        node_type: format!("{:?}", node.node_type),
                        sources: node.sources.iter().take(12).cloned().collect(),
                        aliases: node.aliases.iter().take(12).cloned().collect(),
                        meaning: node.meaning.clone(),
                        context,
                    }
                })
                .collect::<Vec<_>>();
            (nodes, edges)
        };
        if nodes.is_empty() {
            return Ok(GraphCleanupResult {
                proposed_groups: 0,
                proposed_node_refinements: 0,
                proposed_relation_refinements: 0,
                merged_groups: 0,
                merged_nodes: 0,
                renamed_nodes: 0,
                updated_meanings: 0,
                updated_node_types: 0,
                relabeled_relations: 0,
                deduplicated_edges: 0,
                skipped_groups: 0,
                groups: Vec::new(),
                node_refinements: Vec::new(),
                relation_refinements: Vec::new(),
            });
        }

        let _ = events.emit(
            "kb-cleanup-progress",
            serde_json::json!({"step": "analyzing", "pct": 15}),
        );
        let analyzer = GraphCleanupAnalyzer::new(self.manager.config.clone());
        let analysis = analyzer
            .analyze(&nodes, &edges, events)
            .await
            .map_err(BackendError::from)?;
        let checkpoint_signature = analysis.checkpoint_signature.clone();
        let proposed_groups = analysis.merge_groups.len();
        let proposed_node_refinements = analysis.node_refinements.len();
        let proposed_relation_refinements = analysis.relation_refinements.len();
        let _ = events.emit(
            "kb-cleanup-progress",
            serde_json::json!({
                "step": "refining",
                "pct": 75,
                "groups": proposed_groups,
                "nodes": proposed_node_refinements,
                "relations": proposed_relation_refinements
            }),
        );

        let mut graph = self.manager.graph.lock().await;
        let mut working = graph.clone();
        let before_edges = working.edge_count();
        let node_types: HashMap<String, NodeType> = working
            .all_nodes()
            .into_iter()
            .map(|node| (node.name.to_lowercase(), node.node_type.clone()))
            .collect();
        let mut merged_groups = Vec::new();
        let mut merged_nodes = 0;
        let mut renamed_nodes = 0;
        let mut updated_meanings = 0;
        let mut updated_node_types = 0;
        let mut relabeled_relations = 0;
        let mut skipped_groups = 0;
        let mut name_resolution: HashMap<String, String> = HashMap::new();
        let mut applied_node_refinements = Vec::new();
        let mut applied_relation_refinements = Vec::new();

        for proposal in analysis.merge_groups {
            if !proposal.confidence.is_finite() || proposal.confidence < 0.90 {
                skipped_groups += 1;
                continue;
            }
            let canonical_type = match proposal.canonical_type.as_str() {
                "Person" => NodeType::Person,
                "Project" => NodeType::Project,
                "Tool" => NodeType::Tool,
                "Concept" => NodeType::Concept,
                "Other" => NodeType::Other,
                _ => {
                    skipped_groups += 1;
                    continue;
                }
            };
            let canonical = proposal.canonical.trim().to_lowercase();
            let mut names: HashSet<String> = proposal
                .aliases
                .iter()
                .map(|alias| alias.trim().to_lowercase())
                .filter(|alias| !alias.is_empty())
                .collect();
            names.insert(canonical.clone());
            let existing_names: Vec<String> = names
                .into_iter()
                .filter(|name| node_types.contains_key(name))
                .collect();
            let contains_document = existing_names
                .iter()
                .any(|name| node_types.get(name) == Some(&NodeType::Document));
            if canonical.is_empty() || existing_names.len() < 2 || contains_document {
                skipped_groups += 1;
                continue;
            }

            let removed =
                working.merge_node_group(&canonical, &existing_names, Some(canonical_type));
            if removed == 0 {
                skipped_groups += 1;
                continue;
            }
            merged_nodes += removed;
            for alias in &existing_names {
                name_resolution.insert(alias.to_lowercase(), canonical.clone());
            }
            merged_groups.push(GraphCleanupGroup {
                canonical,
                aliases: existing_names,
                confidence: proposal.confidence,
                reason: proposal.reason,
            });
        }

        let mut node_refinement_proposals = analysis.node_refinements;
        node_refinement_proposals.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut refined_targets = HashSet::new();
        for proposal in node_refinement_proposals {
            if !proposal.confidence.is_finite() || proposal.confidence < 0.90 {
                skipped_groups += 1;
                continue;
            }
            let raw_current = proposal.current_name.trim().to_lowercase();
            let current = resolve_graph_name(&raw_current, &name_resolution);
            // An alias that was already merged must not rename the surviving
            // canonical node based on an obsolete alias-specific proposal.
            if current != raw_current || !refined_targets.insert(current.clone()) {
                continue;
            }
            let Some(existing) = working
                .all_nodes()
                .into_iter()
                .find(|node| node.name.eq_ignore_ascii_case(&current))
                .cloned()
            else {
                skipped_groups += 1;
                continue;
            };
            if existing.node_type == NodeType::Document {
                skipped_groups += 1;
                continue;
            }
            let Some(canonical_type) = cleanup_node_type(&proposal.canonical_type) else {
                skipped_groups += 1;
                continue;
            };
            let canonical = proposal.canonical_name.trim().to_lowercase();
            let name_changed = !canonical.is_empty() && canonical != current;
            let meaning_changed =
                !proposal.meaning.trim().is_empty() && existing.meaning != proposal.meaning.trim();
            let type_changed = existing.node_type != canonical_type;
            if !working.refine_node(
                &current,
                &canonical,
                Some(canonical_type),
                &proposal.meaning,
            ) {
                continue;
            }
            if name_changed {
                renamed_nodes += 1;
                name_resolution.insert(current.clone(), canonical.clone());
            }
            if meaning_changed {
                updated_meanings += 1;
            }
            if type_changed {
                updated_node_types += 1;
            }
            applied_node_refinements.push(GraphNodeRefinement {
                previous_name: existing.name,
                canonical_name: if canonical.is_empty() {
                    current
                } else {
                    canonical
                },
                meaning: proposal.meaning.trim().to_string(),
                confidence: proposal.confidence,
                reason: proposal.reason,
            });
        }

        for proposal in analysis.relation_refinements {
            if !proposal.confidence.is_finite() || proposal.confidence < 0.92 {
                skipped_groups += 1;
                continue;
            }
            let from = resolve_graph_name(&proposal.from, &name_resolution);
            let to = resolve_graph_name(&proposal.to, &name_resolution);
            let Some(improved_relation) =
                crate::memory::graph::normalize_relation_label(&proposal.improved_relation)
            else {
                skipped_groups += 1;
                continue;
            };
            if improved_relation == "RELATED_TO"
                && !proposal.current_relation.eq_ignore_ascii_case("RELATED_TO")
            {
                skipped_groups += 1;
                continue;
            }
            if !working.refine_relation(&from, &to, &proposal.current_relation, &improved_relation)
            {
                continue;
            }
            relabeled_relations += 1;
            applied_relation_refinements.push(GraphRelationRefinement {
                from,
                to,
                previous_relation: proposal.current_relation,
                improved_relation,
                confidence: proposal.confidence,
                reason: proposal.reason,
            });
        }

        if merged_nodes > 0
            || renamed_nodes > 0
            || updated_meanings > 0
            || updated_node_types > 0
            || relabeled_relations > 0
        {
            working.sync().await.map_err(BackendError::from)?;
            *graph = working;
        }
        let deduplicated_edges = before_edges.saturating_sub(graph.edge_count());
        let _ = events.emit(
            "kb-cleanup-progress",
            serde_json::json!({
                "step": "complete",
                "pct": 100,
                "merged_nodes": merged_nodes,
                "renamed_nodes": renamed_nodes,
                "updated_meanings": updated_meanings,
                "updated_node_types": updated_node_types,
                "relabeled_relations": relabeled_relations
            }),
        );
        drop(graph);
        analyzer.clear_checkpoint(&checkpoint_signature).await;
        Ok(GraphCleanupResult {
            proposed_groups,
            proposed_node_refinements,
            proposed_relation_refinements,
            merged_groups: merged_groups.len(),
            merged_nodes,
            renamed_nodes,
            updated_meanings,
            updated_node_types,
            relabeled_relations,
            deduplicated_edges,
            skipped_groups,
            groups: merged_groups,
            node_refinements: applied_node_refinements,
            relation_refinements: applied_relation_refinements,
        })
    }

    pub async fn run_quick_extract(
        &self,
        file_path: &str,
    ) -> Result<QuickExtractResult, BackendError> {
        let validated = validate_source_file(file_path)?;
        let filename = validated
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(file_path)
            .to_string();
        log::info!("QuickExtract: processing '{}'", filename);
        let validated = validated.to_string_lossy().to_string();
        let text = tokio::task::spawn_blocking(move || {
            crate::tools::quick_extract::read_file_content(&validated)
        })
        .await
        .map_err(|error| {
            BackendError::new(
                "quick_extract_task",
                format!("QuickExtract task panicked: {error}"),
            )
        })?
        .map_err(BackendError::from)?;
        log::info!(
            "QuickExtract: extracted {} chars from '{}'",
            text.len(),
            filename
        );
        let chunks = crate::tools::quick_extract::chunk_text(&text, 1500).len();
        Ok(QuickExtractResult {
            chars: text.len(),
            chunks,
            text,
            filename,
        })
    }

    pub async fn run_ocr(
        &self,
        file_path: &str,
        knowledge_mode: Option<&str>,
        resources: &ResourceLocator,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<OcrResult, BackendError> {
        let validated = validate_source_file(file_path)?;
        let file_path = validated.to_string_lossy().to_string();
        let pid = uuid::Uuid::new_v4().to_string();
        log::info!("OCR [{}]: starting for {}", pid, file_path);

        let configured_mode = knowledge_mode.unwrap_or(&self.manager.config.ocr_knowledge_mode);
        let knowledge_mode =
            ayla_ocr::OcrKnowledgeMode::parse(configured_mode).ok_or_else(|| {
                BackendError::new(
                    "invalid_ocr_knowledge_mode",
                    "OCR knowledge mode must be disabled, deterministic, or semantic",
                )
            })?;
        let revision = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(client) => match client
                .get(format!("{}/api/tags", self.manager.config.ollama_url))
                .send()
                .await
            {
                Ok(response) => response.json::<Value>().await.ok().and_then(|value| {
                    value
                        .get("models")
                        .and_then(Value::as_array)
                        .and_then(|models| {
                            models.iter().find(|model| {
                                model.get("name").and_then(Value::as_str)
                                    == Some(self.manager.config.ocr_model.as_str())
                            })
                        })
                        .and_then(|model| model.get("digest").and_then(Value::as_str))
                        .map(str::to_string)
                }),
                Err(_) => None,
            },
            Err(_) => None,
        };
        let options = ayla_ocr::OcrOptions {
            model: self.manager.config.ocr_model.clone(),
            model_revision: revision.clone(),
            pdfium_library: Some(
                resources
                    .resolve_resource("lib/libpdfium.so", "target/native/pdfium/lib/libpdfium.so"),
            ),
            ollama_url: self.manager.config.ollama_url.clone(),
            parallel_pages: self.manager.config.ocr_parallel,
            num_ctx: self.manager.config.ocr_num_ctx as u32,
            request_timeout_secs: self.manager.config.ollama_long_timeout_secs as u64,
            resume_cache: self.manager.config.ocr_resume_cache && revision.is_some(),
            cache_dir: Some(self.manager.config.ocr_cache_dir.clone()),
            knowledge_mode,
            assembly_model: Some(self.manager.config.ocr_assembly_model.clone()),
            assembly_allow_cloud: self.manager.config.ocr_assembly_allow_cloud,
            ..Default::default()
        };
        let cancel_rx = processes.register_task(pid.clone()).await;
        let _process_cleanup = processes.cleanup_on_drop(pid.clone());
        let _ = events.emit("ocr-started", serde_json::json!({ "pid": pid }));
        let progress_pid = pid.clone();
        let progress = |update: ayla_ocr::OcrProgress| {
            let mut payload = serde_json::to_value(update).unwrap_or_default();
            payload["pid"] = Value::String(progress_pid.clone());
            let _ = events.emit("ocr-progress", payload);
        };
        let document = tokio::select! {
            result = ayla_ocr::extract_pdf(&file_path, options, Some(&progress)) => {
                processes.finish(&pid).await;
                result.map_err(|error| BackendError::new("ocr_failed", error.to_string()))?
            }
            _ = cancel_rx => {
                processes.finish(&pid).await;
                return Err(BackendError::new("ocr_cancelled", "OCR cancelled"));
            }
        };
        let knowledge_batch = GraphImportBatch::from_ocr_knowledge(
            format!("ocr:{pid}"),
            &document.knowledge,
            &chrono::Utc::now().to_rfc3339(),
        );
        let knowledge_validation = knowledge_batch.validate();
        if !knowledge_validation.is_valid() {
            return Err(BackendError::new(
                "ocr_knowledge_invalid",
                format!(
                    "OCR knowledge preview failed validation: {}",
                    knowledge_validation.errors.join("; ")
                ),
            ));
        }
        let artifacts = crate::artifacts::persist_ocr_artifacts(&pid, &document)?;
        log::info!("OCR [{}]: done — {} chars", pid, document.text.len());
        Ok(OcrResult {
            chars: document.text.len(),
            pages: document.pages_processed.to_string(),
            pid,
            source_name: document.source_name,
            pages_processed: document.pages_processed,
            pages_skipped: document.pages_skipped,
            total_pages: document.total_pages,
            page_results: document.page_results,
            models: document.models,
            warnings: document.warnings,
            knowledge_mode: document.knowledge_mode,
            performance: document.performance,
            cache_key: document.cache_key,
            markdown_artifact: artifacts.markdown,
            layout_artifact: artifacts.layout,
            knowledge_base_artifact: artifacts.knowledge_base,
            structured_artifact: artifacts.structured,
            knowledge_entity_count: document.knowledge.entities.len(),
            knowledge_relation_count: document.knowledge.relations.len(),
            knowledge_validation,
            text: document.text,
        })
    }

    pub async fn ingest_ocr_knowledge(
        &self,
        operation_id: &str,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<OcrKnowledgeIngestion, BackendError> {
        let cancel_rx = processes.register_task(operation_id.into()).await;
        let _process_cleanup = processes.cleanup_on_drop(operation_id);
        let ingestion = async {
            let knowledge = crate::artifacts::load_ocr_knowledge_result(operation_id)?;
            let markdown = knowledge.knowledge_base_markdown;
            validate_ingest_payload_size(markdown.len())?;
            let chars = markdown.chars().count();
            let message = self
                .commit_document(
                    knowledge.source_name.clone(),
                    format!("ocr://{operation_id}/{}", knowledge.source_name),
                    chars.min(u32::MAX as usize) as u32,
                    "ocr_knowledge".into(),
                    markdown,
                    events,
                )
                .await?;
            Ok(OcrKnowledgeIngestion {
                operation_id: operation_id.into(),
                source_name: knowledge.source_name,
                chars,
                message,
            })
        };
        let result = tokio::select! {
            result = ingestion => result,
            _ = cancel_rx => Err(BackendError::new(
                "ocr_knowledge_cancelled",
                "OCR knowledge ingestion cancelled",
            )),
        };
        processes.finish(operation_id).await;
        result
    }

    pub async fn approve_ocr_graph(
        &self,
        operation_id: &str,
    ) -> Result<OcrGraphMutation, BackendError> {
        let knowledge = crate::artifacts::load_ocr_knowledge_result(operation_id)?;
        let source_id = format!("ocr:{operation_id}");
        let batch = GraphImportBatch::from_ocr_knowledge(
            source_id.clone(),
            &knowledge,
            &chrono::Utc::now().to_rfc3339(),
        );
        let validation = batch.validate();
        if !validation.is_valid() {
            return Err(BackendError::new(
                "ocr_knowledge_invalid",
                format!(
                    "OCR graph-v2 import rejected: {}",
                    validation.errors.join("; ")
                ),
            ));
        }
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let receipt = store
            .lock()
            .await
            .import(batch)
            .await
            .map_err(|error| BackendError::new("graph_v2_import", error))?;
        Ok(OcrGraphMutation {
            operation_id: operation_id.into(),
            source_id,
            receipt,
        })
    }

    pub async fn remove_ocr_graph(
        &self,
        operation_id: &str,
    ) -> Result<OcrGraphMutation, BackendError> {
        let _ = crate::artifacts::load_ocr_knowledge_result(operation_id)?;
        let source_id = format!("ocr:{operation_id}");
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let receipt = store
            .lock()
            .await
            .remove_source(&source_id)
            .await
            .map_err(|error| BackendError::new("graph_v2_remove", error))?;
        Ok(OcrGraphMutation {
            operation_id: operation_id.into(),
            source_id,
            receipt,
        })
    }

    pub async fn run_code_analysis(
        &self,
        mut request: CodeAnalysisRequest,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<CodeAnalysisResult, BackendError> {
        let parent = request
            .parent_operation_id
            .as_deref()
            .map(crate::artifacts::load_code_analysis_run)
            .transpose()?;
        if let Some(parent) = &parent {
            if request.source_type == "snippet" {
                if request.name.is_none() {
                    request.name = Some(parent.source_label.clone());
                }
                if request.language.is_none() {
                    request.language = parent.language.clone();
                }
                if request
                    .code
                    .as_deref()
                    .map(str::trim)
                    .map_or(true, str::is_empty)
                    && parent.source_snapshot_stored
                {
                    request.code = Some(crate::artifacts::load_code_analysis_private_source(
                        &parent.operation_id,
                    )?);
                }
            }
        }
        if parent
            .as_ref()
            .is_some_and(|parent| parent.source_type != request.source_type)
        {
            return Err(BackendError::new(
                "code_analysis_parent_mismatch",
                "Re-analysis must use the same source type as its parent run",
            ));
        }
        if let Some(parent) = &parent {
            validate_code_analysis_parent(&request, parent)?;
        }
        let source = validate_code_analysis_source(&request)?;
        let mut options = ayla_code_analysis::GenerateOptions::default();
        options.allow_remote_git = false;
        options.allow_ssh_git = false;
        options.output_name = request.name.clone();
        options.source_scope = match request.scope.as_deref().unwrap_or("documents") {
            "documents" | "documents_only" => ayla_code_analysis::SourceScope::DocumentsOnly,
            "all" => ayla_code_analysis::SourceScope::All,
            _ => {
                return Err(BackendError::new(
                    "invalid_code_analysis_scope",
                    "Code-analysis scope must be 'documents' or 'all'",
                ))
            }
        };
        let semantic_mode = request
            .semantic_mode
            .as_deref()
            .unwrap_or("deterministic")
            .trim()
            .to_ascii_lowercase();
        match semantic_mode.as_str() {
            "deterministic" => options.semantic_analysis = false,
            "local" | "ollama" => {
                let model = self.manager.config.code_analysis_model.trim();
                let cloud_model = model.to_ascii_lowercase().contains(":cloud");
                if model.is_empty() {
                    return Err(BackendError::new(
                        "local_semantic_model_invalid",
                        "Semantic enrichment requires AYLA_CODE_ANALYSIS_MODEL",
                    ));
                }
                if cloud_model && semantic_mode == "local" {
                    return Err(BackendError::new(
                        "local_semantic_model_invalid",
                        "Local semantic enrichment cannot use a cloud model; choose configured Ollama mode",
                    ));
                }
                if cloud_model && !self.manager.config.code_analysis_allow_cloud {
                    return Err(BackendError::new(
                        "code_semantic_cloud_disabled",
                        "Cloud code analysis requires AYLA_CODE_ANALYSIS_ALLOW_CLOUD=true",
                    ));
                }
                options.ollama_base_url =
                    self.manager.config.ollama_url.parse().map_err(|error| {
                        BackendError::new(
                            "local_semantic_url_invalid",
                            format!("The configured Ollama URL is invalid: {error}"),
                        )
                    })?;
                let local_host = matches!(
                    options.ollama_base_url.host_str(),
                    Some("localhost" | "127.0.0.1" | "::1")
                );
                let local_transport = matches!(options.ollama_base_url.scheme(), "http" | "https")
                    && options.ollama_base_url.username().is_empty()
                    && options.ollama_base_url.password().is_none();
                if !local_host || !local_transport {
                    return Err(BackendError::new(
                        "local_semantic_remote_blocked",
                        "Semantic enrichment only permits an unauthenticated HTTP(S) loopback Ollama endpoint",
                    ));
                }
                options.model = model.to_string();
                options.semantic_analysis = true;
            }
            _ => {
                return Err(BackendError::new(
                    "invalid_code_semantic_mode",
                    "Semantic mode must be 'deterministic', 'local', or 'ollama'",
                ))
            }
        }

        let pid = uuid::Uuid::new_v4().to_string();
        let cancel_rx = processes.register_task(pid.clone()).await;
        let _process_cleanup = processes.cleanup_on_drop(pid.clone());
        let _ = events.emit("code-analysis-started", serde_json::json!({ "pid": pid }));
        let mut cancel_rx = cancel_rx;
        let documents = if semantic_mode != "deterministic" {
            let _ = events.emit(
                "code-analysis-progress",
                serde_json::json!({
                    "pid": pid,
                    "stage": "semantic",
                    "current": 0,
                    "total": 1,
                    "message": "Analyzing structure with the configured Ollama semantic model"
                }),
            );
            let generator = ayla_code_analysis::Generator::with_llm(
                ayla_code_analysis::OllamaClient::default(),
            );
            let mut worker = tokio::spawn(async move {
                ayla_code_analysis::CodeDocumentationGenerator::generate(
                    &generator, source, options,
                )
                .await
            });
            let _worker_guard = CodeAnalysisAsyncWorkerGuard(worker.abort_handle());
            tokio::select! {
                result = &mut worker => result.map_err(|error| BackendError::new(
                    "code_analysis_task",
                    format!("Code-analysis task panicked: {error}"),
                ))?.map_err(code_analysis_error)?,
                _ = &mut cancel_rx => {
                    worker.abort();
                    let _ = worker.await;
                    processes.finish(&pid).await;
                    return Err(BackendError::new(
                        "code_analysis_cancelled",
                        "Code analysis cancelled",
                    ));
                }
            }
        } else {
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
            let control = ayla_code_analysis::AnalysisControl::with_progress(move |update| {
                // The API emits the terminal event synchronously before the
                // response, avoiding a worker/channel race and duplicates.
                if update.stage != "complete" {
                    let _ = progress_tx.send(update);
                }
            });
            // Renderer/request cancellation may drop this future before its select
            // branch runs. The guard still signals the blocking parser to stop.
            let _worker_guard = CodeAnalysisWorkerGuard(control.clone());
            let worker_control = control.clone();
            let mut worker = tokio::task::spawn_blocking(move || {
                ayla_code_analysis::generate_deterministic(source, options, &worker_control)
            });
            let mut progress_open = true;
            loop {
                tokio::select! {
                    result = &mut worker => {
                        break result.map_err(|error| BackendError::new(
                            "code_analysis_task",
                            format!("Code-analysis task panicked: {error}"),
                        ))?.map_err(code_analysis_error)?;
                    }
                    _ = &mut cancel_rx => {
                        control.cancel();
                        let _ = worker.await;
                        processes.finish(&pid).await;
                        return Err(BackendError::new(
                            "code_analysis_cancelled",
                            "Code analysis cancelled",
                        ));
                    }
                    progress = progress_rx.recv(), if progress_open => {
                        match progress {
                            Some(progress) => {
                                let mut payload = serde_json::to_value(progress).unwrap_or_default();
                                payload["pid"] = Value::String(pid.clone());
                                let _ = events.emit("code-analysis-progress", payload);
                            }
                            None => progress_open = false,
                        }
                    }
                }
            }
        };
        processes.finish(&pid).await;
        let _ = events.emit(
            "code-analysis-progress",
            serde_json::json!({
                "pid": pid,
                "stage": "complete",
                "current": 1,
                "total": 1,
                "message": "Code analysis complete"
            }),
        );

        let timestamp = chrono::Utc::now().to_rfc3339();
        let batch = GraphImportBatch::from_code_analysis(
            format!("code-analysis:{pid}"),
            &documents,
            &timestamp,
        );
        let validation = batch.validate();
        if !validation.is_valid() {
            return Err(BackendError::new(
                "code_analysis_graph_invalid",
                format!(
                    "Generated graph preview failed validation: {}",
                    validation.errors.join("; ")
                ),
            ));
        }
        let mut artifacts = crate::artifacts::persist_code_analysis_artifacts(&pid, &documents)?;
        let diff = if let Some(parent) = &parent {
            let previous = crate::artifacts::load_code_analysis_result(&parent.operation_id)?;
            let (summary, artifact) = crate::artifacts::persist_code_analysis_diff(
                &pid,
                &parent.operation_id,
                &documents,
                &previous,
            )?;
            artifacts.diff = Some(artifact);
            Some(summary)
        } else {
            None
        };
        let source_snapshot_stored = if request.source_type == "snippet" {
            let source = request.code.as_deref().ok_or_else(|| {
                BackendError::new(
                    "private_source_missing",
                    "The analyzed snippet source is unavailable for private persistence",
                )
            })?;
            crate::artifacts::persist_code_analysis_private_source(&pid, source)?;
            true
        } else {
            false
        };
        let source_label = if request.source_type == "directory" {
            request
                .directory_path
                .as_deref()
                .and_then(|path| std::path::Path::new(path).file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("Local directory")
                .to_string()
        } else {
            request.name.clone().unwrap_or_else(|| "snippet.rs".into())
        };
        let run = crate::artifacts::CodeAnalysisRunRecord {
            schema_version: 2,
            operation_id: pid.clone(),
            parent_operation_id: parent.as_ref().map(|run| run.operation_id.clone()),
            created_at: timestamp.clone(),
            updated_at: timestamp.clone(),
            status: "preview".into(),
            source_type: request.source_type.clone(),
            source_label,
            directory_path: request.directory_path.clone(),
            language: request.language.clone(),
            scope: request.scope.clone().unwrap_or_else(|| "documents".into()),
            semantic_mode: semantic_mode.clone(),
            project_name: documents.project_name.clone(),
            repository_uid: documents.repository_uid.clone(),
            entity_count: documents.entities.len(),
            relation_count: documents.relations.len(),
            diagnostics: documents.diagnostics.clone(),
            artifacts: artifacts.clone(),
            diff: diff.clone(),
            graph_approved: false,
            vectors_indexed: false,
            source_snapshot_stored,
        };
        crate::artifacts::persist_code_analysis_run(&run)?;
        Ok(CodeAnalysisResult {
            pid,
            project_name: documents.project_name,
            repository_uid: documents.repository_uid,
            entity_count: documents.entities.len(),
            relation_count: documents.relations.len(),
            diagnostics: documents.diagnostics,
            validation,
            artifacts,
            parent_operation_id: run.parent_operation_id,
            created_at: timestamp,
            semantic_mode,
            diff,
            graph_approved: false,
            vectors_indexed: false,
            source_snapshot_stored,
        })
    }

    pub fn get_code_analysis_runs(
        &self,
    ) -> Result<Vec<crate::artifacts::CodeAnalysisRunRecord>, BackendError> {
        crate::artifacts::list_code_analysis_runs()
    }

    pub fn get_code_analysis_run(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisResult, BackendError> {
        let run = crate::artifacts::load_code_analysis_run(operation_id)?;
        let documents = crate::artifacts::load_code_analysis_result(operation_id)?;
        let batch = GraphImportBatch::from_code_analysis(
            format!("code-analysis:{operation_id}"),
            &documents,
            &run.updated_at,
        );
        let validation = batch.validate();
        Ok(CodeAnalysisResult {
            pid: run.operation_id,
            project_name: run.project_name,
            repository_uid: run.repository_uid,
            entity_count: run.entity_count,
            relation_count: run.relation_count,
            diagnostics: run.diagnostics,
            validation,
            artifacts: run.artifacts,
            parent_operation_id: run.parent_operation_id,
            created_at: run.created_at,
            semantic_mode: run.semantic_mode,
            diff: run.diff,
            graph_approved: run.graph_approved,
            vectors_indexed: run.vectors_indexed,
            source_snapshot_stored: run.source_snapshot_stored,
        })
    }

    pub fn get_artifact_content(
        &self,
        artifact_id: &str,
    ) -> Result<crate::ArtifactContent, BackendError> {
        crate::artifacts::read_artifact_content(artifact_id)
    }

    pub async fn approve_code_analysis(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisApproval, BackendError> {
        let run = crate::artifacts::load_code_analysis_run(operation_id)?;
        if run.graph_approved {
            return Err(BackendError::new(
                "code_analysis_already_approved",
                "This code-analysis run is already approved",
            ));
        }
        let approved_parent = run
            .parent_operation_id
            .as_deref()
            .and_then(|parent_id| crate::artifacts::load_code_analysis_run(parent_id).ok())
            .filter(|parent| parent.graph_approved);
        let documents = crate::artifacts::load_code_analysis_result(operation_id)?;
        let source_id = format!("code-analysis:{operation_id}");
        let batch = GraphImportBatch::from_code_analysis(
            source_id.clone(),
            &documents,
            &chrono::Utc::now().to_rfc3339(),
        );
        let validation = batch.validate();
        if !validation.is_valid() {
            return Err(BackendError::new(
                "code_analysis_graph_invalid",
                format!("Graph-v2 import rejected: {}", validation.errors.join("; ")),
            ));
        }
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let vector_store = self.manager.code_analysis_vectors.as_ref().ok_or_else(|| {
            BackendError::new(
                "code_analysis_vectors_offline",
                "Code-analysis vector storage is unavailable while Qdrant is offline",
            )
        })?;
        let vector_receipt = match vector_store.index_run(operation_id, &documents).await {
            Ok(receipt) => receipt,
            Err(error) => {
                return Err(BackendError::new(
                    "code_analysis_vector_index",
                    format!("Could not index code-analysis vectors: {error}"),
                ));
            }
        };
        let graph_result = if let Some(parent) = &approved_parent {
            store
                .lock()
                .await
                .replace_source(&format!("code-analysis:{}", parent.operation_id), batch)
                .await
        } else {
            store.lock().await.import(batch).await
        };
        let receipt = match graph_result {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = vector_store.delete_run(operation_id).await;
                return Err(BackendError::new("graph_v2_import", error));
            }
        };
        if let Err(error) = crate::artifacts::update_code_analysis_run(operation_id, |run| {
            run.status = "approved".into();
            run.graph_approved = true;
            run.vectors_indexed = true;
        }) {
            let _ = vector_store.delete_run(operation_id).await;
            let _ = store.lock().await.rollback(&receipt.backup_id).await;
            return Err(error);
        }
        if let Some(parent) = &approved_parent {
            if let Err(error) =
                crate::artifacts::update_code_analysis_run(&parent.operation_id, |run| {
                    run.status = "superseded".into();
                    run.graph_approved = false;
                })
            {
                let _ = vector_store.delete_run(operation_id).await;
                let _ = store.lock().await.rollback(&receipt.backup_id).await;
                let _ = crate::artifacts::update_code_analysis_run(operation_id, |run| {
                    run.status = "preview".into();
                    run.graph_approved = false;
                    run.vectors_indexed = false;
                });
                return Err(error);
            }
        }
        Ok(CodeAnalysisApproval {
            operation_id: operation_id.into(),
            source_id,
            receipt,
            entity_vectors: vector_receipt.entity_vectors,
            document_vectors: vector_receipt.document_vectors,
        })
    }

    pub async fn remove_code_analysis(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisRemoval, BackendError> {
        // Requiring the corresponding artifact prevents this general-purpose
        // endpoint from being used to remove arbitrary provenance IDs.
        let _ = crate::artifacts::load_code_analysis_result(operation_id)?;
        let source_id = format!("code-analysis:{operation_id}");
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let receipt = store
            .lock()
            .await
            .remove_source(&source_id)
            .await
            .map_err(|error| BackendError::new("graph_v2_remove", error))?;
        crate::artifacts::update_code_analysis_run(operation_id, |run| {
            run.status = "preview".into();
            run.graph_approved = false;
        })?;
        Ok(CodeAnalysisRemoval {
            operation_id: operation_id.into(),
            source_id,
            receipt,
        })
    }

    pub async fn delete_code_analysis_run(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisRunDeletion, BackendError> {
        let run = crate::artifacts::load_code_analysis_run(operation_id)?;
        let graph_store = if run.graph_approved {
            Some(self.manager.graph_v2.as_ref().ok_or_else(|| {
                BackendError::new(
                    "graph_v2_offline",
                    "Cannot delete an approved run while graph-v2 storage is offline",
                )
            })?)
        } else {
            self.manager.graph_v2.as_ref()
        };
        let vector_store = if run.vectors_indexed {
            Some(self.manager.code_analysis_vectors.as_ref().ok_or_else(|| {
                BackendError::new(
                    "code_analysis_vectors_offline",
                    "Cannot delete an indexed run while Qdrant is offline",
                )
            })?)
        } else {
            self.manager.code_analysis_vectors.as_ref()
        };

        crate::artifacts::update_code_analysis_run(operation_id, |run| {
            run.status = "deleting".into();
        })?;

        let mut graph_removed = false;
        if let Some(store) = graph_store {
            store
                .lock()
                .await
                .remove_source(&format!("code-analysis:{operation_id}"))
                .await
                .map_err(|error| BackendError::new("graph_v2_remove", error))?;
            graph_removed = true;
            crate::artifacts::update_code_analysis_run(operation_id, |run| {
                run.graph_approved = false;
            })?;
        }

        let mut vectors_removed = false;
        if let Some(store) = vector_store {
            if let Err(error) = store.delete_run(operation_id).await {
                let _ = crate::artifacts::update_code_analysis_run(operation_id, |run| {
                    run.status = "delete_failed".into();
                });
                return Err(BackendError::new(
                    "code_analysis_vector_delete",
                    format!("Could not delete code-analysis vectors: {error}"),
                ));
            }
            vectors_removed = true;
            crate::artifacts::update_code_analysis_run(operation_id, |run| {
                run.vectors_indexed = false;
            })?;
        }

        if let Err(error) = crate::artifacts::delete_code_analysis_artifacts(operation_id) {
            let _ = crate::artifacts::update_code_analysis_run(operation_id, |run| {
                run.status = "delete_failed".into();
            });
            return Err(error);
        }
        Ok(CodeAnalysisRunDeletion {
            operation_id: operation_id.into(),
            graph_removed,
            vectors_removed,
            artifacts_removed: true,
        })
    }

    pub async fn run_search(
        &self,
        query: String,
        mode: Option<String>,
        resources: &ResourceLocator,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<Value, BackendError> {
        crate::research::run_search(self.manager, resources, events, processes, query, mode).await
    }

    pub async fn load_chat_sessions(&self) -> Result<Vec<ChatSessionRecord>, BackendError> {
        if !self.manager.is_db_ready() {
            return Ok(Vec::new());
        }

        let db = self.connect_database().await?;
        db.ensure_table("chat_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("chat_message")
            .await
            .map_err(BackendError::from)?;
        let sessions = db
            .query("SELECT * FROM chat_session ORDER BY updated_at DESC;")
            .await
            .map_err(BackendError::from)?;
        let mut result = Vec::with_capacity(sessions.len());
        for session in sessions {
            let session_id = string_field(&session, "session_id");
            if session_id.is_empty() {
                continue;
            }
            let mut variables = serde_json::Map::new();
            variables.insert("sid".into(), Value::String(session_id.clone()));
            let messages = db
                .query_with_vars(
                    "SELECT * FROM chat_message WHERE session_id = $sid ORDER BY timestamp ASC;",
                    variables,
                )
                .await
                .map_err(BackendError::from)?
                .iter()
                .map(|message| ChatMessageRecord {
                    role: string_field(message, "role"),
                    content: string_field(message, "content"),
                })
                .collect();
            result.push(ChatSessionRecord {
                id: session_id,
                title: string_field(&session, "title"),
                messages,
            });
        }
        Ok(result)
    }

    pub async fn get_search_history(&self) -> Result<Vec<SearchHistoryRecord>, BackendError> {
        if !self.manager.is_db_ready() {
            return Ok(Vec::new());
        }

        let db = self.connect_search_database().await?;
        db.ensure_table("search_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("search_document")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        // Avoid selecting the legacy report/text blobs. Older records can
        // contain NUL characters that SurrealDB 2.1 cannot serialize safely.
        let sessions = db
            .query(
                "SELECT search_id, query, mode, status, warnings, model, source_outcomes, created_at, updated_at, knowledge_document, knowledge_ingested, synthesis, papers, total_results_found, pages_rendered FROM search_session ORDER BY created_at DESC LIMIT 50;",
            )
            .await
            .map_err(BackendError::from)?;
        let documents = db
            .query(
                "SELECT document_id, search_id, name, path, source_url, mime_type, size_bytes, status, extraction_method, ocr_operation_id, chars, chunks, processed_at, ingested_at, created_at FROM search_document ORDER BY created_at ASC;",
            )
            .await
            .map_err(BackendError::from)?;
        let ingested_knowledge_paths: HashSet<String> = db
            .query("SELECT path FROM kb_file WHERE source = 'meta_search_knowledge' AND status = 'committed' AND name NOT IN (SELECT VALUE name FROM knowledge_operation WHERE status != 'committed');")
            .await
            .map_err(BackendError::from)?
            .iter()
            .map(|record| string_field(record, "path"))
            .filter(|path| !path.is_empty())
            .collect();

        Ok(build_search_history(
            &sessions,
            &documents,
            &ingested_knowledge_paths,
        ))
    }

    pub async fn get_search_document_content(
        &self,
        search_id: &str,
        document_id: &str,
    ) -> Result<Value, BackendError> {
        let db = self.connect_search_database().await?;
        db.ensure_table("search_document")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("document_id".into(), Value::String(document_id.to_string()));
        db.query_with_vars(
            "SELECT * FROM search_document WHERE search_id = $search_id AND document_id = $document_id LIMIT 1;",
            variables,
        )
        .await
        .map_err(BackendError::from)?
        .into_iter()
        .next()
        .ok_or_else(|| BackendError::new("not_found", "Search document not found"))
    }

    pub async fn search_documents(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<DocumentSearchResult>, BackendError> {
        let document_store =
            self.manager.document_store.as_ref().ok_or_else(|| {
                BackendError::new("document_store_offline", "Document store offline")
            })?;
        Ok(document_store
            .search(query, limit as u64)
            .await
            .map_err(BackendError::from)?
            .into_iter()
            .map(|(text, score)| DocumentSearchResult { text, score })
            .collect())
    }

    pub async fn save_chat_session(
        &self,
        session_id: &str,
        title: &str,
        messages_json: &str,
    ) -> Result<(), BackendError> {
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        validate_session_payload_size(messages_json.len())?;

        let db = self.connect_database().await?;
        db.ensure_table("chat_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("chat_message")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("sid".into(), Value::String(session_id.to_string()));
        // Preserve the existing replace semantics and plain `session_id`
        // field; SurrealDB's reserved `id` would become a record reference.
        let _ = db
            .exec_with_vars(
                "DELETE chat_message WHERE session_id = $sid;",
                variables.clone(),
            )
            .await;
        let _ = db
            .exec_with_vars("DELETE chat_session WHERE session_id = $sid;", variables)
            .await;
        db.insert(
            "chat_session",
            serde_json::json!({
                "session_id": session_id,
                "title": title,
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }),
        )
        .await
        .map_err(BackendError::from)?;

        let messages: Vec<Value> = serde_json::from_str(messages_json).unwrap_or_default();
        for message in messages {
            db.insert(
                "chat_message",
                serde_json::json!({
                    "session_id": session_id,
                    "role": message["role"],
                    "content": message["content"],
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }),
            )
            .await
            .map_err(BackendError::from)?;
        }
        Ok(())
    }

    pub async fn delete_chat_session(&self, session_id: &str) -> Result<(), BackendError> {
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        let db = self.connect_database().await?;
        db.ensure_table("chat_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("chat_message")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("sid".into(), Value::String(session_id.to_string()));
        let _ = db
            .exec_with_vars(
                "DELETE chat_message WHERE session_id = $sid;",
                variables.clone(),
            )
            .await;
        let _ = db
            .exec_with_vars("DELETE chat_session WHERE session_id = $sid;", variables)
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_search_document_result(
        &self,
        search_id: &str,
        document_id: &str,
        method: &str,
        text: &str,
        chars: u32,
        chunks: u32,
        ocr_operation_id: Option<&str>,
    ) -> Result<(), BackendError> {
        validate_search_document_input(method, text.len())?;

        let db = self.connect_search_database().await?;
        db.ensure_table("search_document")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("document_id".into(), Value::String(document_id.to_string()));
        variables.insert("method".into(), Value::String(method.to_string()));
        variables.insert("text".into(), Value::String(text.to_string()));
        variables.insert("chars".into(), serde_json::json!(chars));
        variables.insert("chunks".into(), serde_json::json!(chunks));
        variables.insert(
            "ocr_operation_id".into(),
            serde_json::json!(ocr_operation_id),
        );
        variables.insert("now".into(), Value::String(chrono::Utc::now().to_rfc3339()));
        db.exec_with_vars(
            "UPDATE search_document SET status = 'processed', extraction_method = $method, ocr_operation_id = $ocr_operation_id, extracted_text = $text, chars = $chars, chunks = $chunks, processed_at = $now, updated_at = $now WHERE search_id = $search_id AND document_id = $document_id RETURN NONE;",
            variables,
        )
        .await
        .map_err(BackendError::from)
    }

    pub async fn mark_search_document_ingested(
        &self,
        search_id: &str,
        document_id: &str,
    ) -> Result<(), BackendError> {
        let db = self.connect_search_database().await?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("document_id".into(), Value::String(document_id.to_string()));
        variables.insert("now".into(), Value::String(chrono::Utc::now().to_rfc3339()));
        db.exec_with_vars(
            "UPDATE search_document SET status = 'ingested', ingested_at = $now, updated_at = $now WHERE search_id = $search_id AND document_id = $document_id RETURN NONE;",
            variables,
        )
        .await
        .map_err(BackendError::from)
    }

    pub async fn mark_search_knowledge_ingested(
        &self,
        search_id: &str,
    ) -> Result<(), BackendError> {
        let db = self.connect_search_database().await?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("now".into(), Value::String(chrono::Utc::now().to_rfc3339()));
        db.exec_with_vars(
            "UPDATE search_session SET knowledge_ingested = true, updated_at = $now WHERE search_id = $search_id RETURN NONE;",
            variables,
        )
        .await
        .map_err(BackendError::from)
    }

    pub async fn commit_document(
        &self,
        name: String,
        path: String,
        chars: u32,
        source_type: String,
        text: String,
        events: &dyn EventSink,
    ) -> Result<String, BackendError> {
        let _knowledge_writer = KNOWLEDGE_WRITER.lock().await;
        validate_file_payload_size(text.len())?;
        let db = self.connect_search_database().await?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        let identity = serde_json::json!({"name": name})
            .as_object()
            .unwrap()
            .clone();
        let existing = db
            .query_with_vars(
                "SELECT path FROM kb_file WHERE name = $name LIMIT 2;",
                identity,
            )
            .await
            .map_err(BackendError::from)?;
        if existing.iter().any(|record| {
            record
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|old| old != path)
        }) {
            return Err(BackendError::new("duplicate_document_name", "A different document already uses this name. Rename it before importing; the existing source was preserved."));
        }
        db.ensure_table("knowledge_operation")
            .await
            .map_err(BackendError::from)?;
        let vars = serde_json::json!({"name": name, "path": path, "text": text, "source_type": source_type, "chars": chars}).as_object().unwrap().clone();
        db.exec_with_vars("BEGIN TRANSACTION; DELETE knowledge_operation WHERE name = $name; CREATE knowledge_operation CONTENT { name: $name, path: $path, text: $text, source_type: $source_type, chars: $chars, status: 'approving' }; COMMIT TRANSACTION;", vars.clone()).await.map_err(BackendError::from)?;
        let committed_text = text.clone();
        let result = async {
            self.save_file_record(name.clone(), path, chars, source_type.clone(), text.clone())
                .await?;
            if source_type == "ocr_knowledge" {
                let operation_id = path_ocr_operation(&vars["path"].as_str().unwrap_or_default())
                    .ok_or_else(|| BackendError::new("invalid_ocr_source", "Missing OCR operation identity"))?;
                self.approve_ocr_graph(operation_id).await?;
                db.ensure_table("text_store").await.map_err(BackendError::from)?;
                db.exec_with_vars("BEGIN TRANSACTION; DELETE text_store WHERE key = $name; CREATE text_store CONTENT { key: $name, value: $text }; COMMIT TRANSACTION;", vars.clone()).await.map_err(BackendError::from)?;
                Ok(format!("Approved OCR evidence, graph, and vectors from {name}"))
            } else {
                {
                    let mut graph = self.manager.graph.lock().await;
                    graph.remove_by_source(&name);
                    graph.sync().await.map_err(BackendError::from)?;
                }
                self.ingest_text(text, name.clone(), source_type, events).await
            }
        }
        .await;
        match result {
            Ok(message) => {
                db.exec_with_vars("UPDATE knowledge_operation SET status = 'committed', text = NONE WHERE name = $name RETURN NONE; UPDATE kb_file SET status = 'committed' WHERE name = $name RETURN NONE;", vars).await.map_err(BackendError::from)?;
                self.manager
                    .store_document_text(name.clone(), committed_text)
                    .await;
                let _ = events.emit(
                    "knowledge-changed",
                    serde_json::json!({"source": name, "action": "commit"}),
                );
                Ok(message)
            }
            Err(error) => {
                self.manager.remove_document_text(&name).await;
                let _ = db.exec_with_vars("UPDATE knowledge_operation SET status = 'failed' WHERE name = $name RETURN NONE; UPDATE kb_file SET status = 'failed' WHERE name = $name RETURN NONE;", vars).await;
                Err(error)
            }
        }
    }

    pub async fn save_file_record(
        &self,
        name: String,
        path: String,
        chars: u32,
        source_type: String,
        text: String,
    ) -> Result<(), BackendError> {
        log::info!(
            "File record: saving '{}' ({} chars, path={})",
            name,
            chars,
            crate::memory::safe_truncate(&path, 40)
        );
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        validate_file_payload_size(text.len())?;

        let db = self.connect_database().await?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;

        let vars = serde_json::json!({"name": name})
            .as_object()
            .unwrap()
            .clone();
        let existing = db
            .query_with_vars("SELECT path FROM kb_file WHERE name = $name LIMIT 2;", vars)
            .await
            .map_err(BackendError::from)?;
        if existing.iter().any(|record| {
            record
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|old| old != path)
        }) {
            return Err(BackendError::new("duplicate_document_name", "A different document already uses this name. Rename the incoming file or remove the existing source before importing it."));
        }
        let source_id = format!(
            "document:{:x}",
            sha2::Sha256::digest(format!("{name}\0{path}").as_bytes())
        );
        // Approval is the retrieval commit point. Discarded quick-extraction
        // previews must never leave orphan vectors behind.
        if let Some(document_store) = &self.manager.document_store {
            let chunks = crate::tools::quick_extract::chunk_text(&text, 1500);
            document_store
                .delete_by_filename(&name)
                .await
                .map_err(BackendError::from)?;
            document_store
                .store_chunks(&name, &chunks)
                .await
                .map_err(BackendError::from)?;
        }

        let mut variables = serde_json::Map::new();
        variables.insert("name".into(), Value::String(name.clone()));
        db.exec_with_vars("DELETE kb_file WHERE name = $name;", variables)
            .await
            .map_err(BackendError::from)?;
        let record = serde_json::json!({
            "name": name,
            "source_id": source_id,
            "status": "committed",
            "path": path,
            "chars": chars,
            "source": source_type,
            "date": chrono::Utc::now().to_rfc3339(),
            "text": text,
        });
        if let Err(error) = db.insert("kb_file", record).await {
            if let Some(document_store) = &self.manager.document_store {
                let _ = document_store.delete_by_filename(&name).await;
            }
            return Err(BackendError::from(error));
        }
        log::info!("File record: saved successfully");
        Ok(())
    }

    pub async fn delete_file_record(
        &self,
        name: &str,
        delete_nodes: bool,
    ) -> Result<String, BackendError> {
        let _knowledge_writer = KNOWLEDGE_WRITER.lock().await;
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        let db = self.connect_database().await?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("text_store")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("name".into(), Value::String(name.to_string()));
        let records = db
            .query_with_vars(
                "SELECT path FROM kb_file WHERE name = $name;",
                variables.clone(),
            )
            .await
            .map_err(BackendError::from)?;
        self.manager.remove_document_text(name).await;
        db.exec_with_vars(
            "UPDATE kb_file SET status = 'deleting' WHERE name = $name RETURN NONE;",
            variables.clone(),
        )
        .await
        .map_err(BackendError::from)?;
        // Do not acknowledge deletion while vectors survive. The durable tombstone
        // keeps the source out of retrieval and makes this operation safe to retry.
        if let Some(document_store) = &self.manager.document_store {
            document_store
                .delete_by_filename(name)
                .await
                .map_err(|e| BackendError::new("document_delete_incomplete", e))?;
        }
        let mut removed_nodes = 0;
        if delete_nodes {
            let mut graph = self.manager.graph.lock().await;
            removed_nodes = graph.remove_by_source(name);
            graph.sync().await.map_err(BackendError::from)?;
            drop(graph);
            for record in records {
                if let Some(operation) = record
                    .get("path")
                    .and_then(Value::as_str)
                    .and_then(path_ocr_operation)
                {
                    if let Some(store) = &self.manager.graph_v2 {
                        store
                            .lock()
                            .await
                            .remove_source(&format!("ocr:{operation}"))
                            .await
                            .map_err(BackendError::from)?;
                    }
                }
            }
        }
        db.exec_with_vars("BEGIN TRANSACTION; DELETE knowledge_operation WHERE name = $name; CREATE knowledge_operation CONTENT { name: $name, status: 'deleted' }; DELETE text_store WHERE key = $name; DELETE kb_file WHERE name = $name; COMMIT TRANSACTION;", variables).await.map_err(BackendError::from)?;
        self.manager.remove_document_text(name).await;
        log::info!(
            "File record: deleted '{}' ({} graph nodes removed)",
            name,
            removed_nodes
        );
        Ok(format!(
            "Deleted '{}' ({} graph nodes removed)",
            name, removed_nodes
        ))
    }

    pub async fn delete_file_nodes(&self, name: &str) -> Result<String, BackendError> {
        let mut graph = self.manager.graph.lock().await;
        let removed_nodes = graph.remove_by_source(name);
        graph.sync().await.map_err(BackendError::from)?;
        Ok(format!("Removed {} nodes from '{}'", removed_nodes, name))
    }

    async fn connect_database(&self) -> Result<SurrealDB, BackendError> {
        let config = &self.manager.config;
        SurrealDB::connect(
            &config.surrealdb_url,
            &config.surrealdb_ns,
            &config.surrealdb_db,
            &config.surrealdb_user,
            &config.surrealdb_pass,
        )
        .await
        .map_err(BackendError::from)
    }

    async fn connect_search_database(&self) -> Result<SurrealDB, BackendError> {
        if !self.manager.is_db_ready() {
            return Err(BackendError::new(
                "db_offline",
                "DB offline: Meta Search history cannot be stored",
            ));
        }
        self.connect_database().await
    }
}

fn path_ocr_operation(path: &str) -> Option<&str> {
    path.strip_prefix("ocr://")?
        .split('/')
        .next()
        .filter(|id| !id.is_empty())
}

fn validate_code_analysis_source(
    request: &CodeAnalysisRequest,
) -> Result<ayla_code_analysis::CodeSource, BackendError> {
    match request.source_type.as_str() {
        "directory" => {
            let value = request.directory_path.as_deref().ok_or_else(|| {
                BackendError::new(
                    "invalid_code_source",
                    "A directory path is required for directory analysis",
                )
            })?;
            if value.len() > 4096 {
                return Err(BackendError::new(
                    "invalid_code_source",
                    "Code-analysis directory path is too long",
                ));
            }
            let path = std::fs::canonicalize(value).map_err(|error| {
                BackendError::new(
                    "invalid_code_source",
                    format!("Could not resolve code-analysis directory: {error}"),
                )
            })?;
            if !path.is_dir() {
                return Err(BackendError::new(
                    "invalid_code_source",
                    "Code-analysis source must be a directory",
                ));
            }
            Ok(ayla_code_analysis::CodeSource::Directory { path })
        }
        "snippet" => {
            let code = request.code.clone().unwrap_or_default();
            if code.trim().is_empty() {
                return Err(BackendError::new(
                    "invalid_code_source",
                    "Code snippet must not be empty",
                ));
            }
            if code.len() > MAX_CODE_SNIPPET_BYTES {
                return Err(BackendError::new(
                    "code_source_too_large",
                    "Code snippet exceeds the 2 MiB safety limit",
                ));
            }
            let name = request.name.clone().unwrap_or_else(|| "snippet.rs".into());
            if name.is_empty() || name.len() > 200 {
                return Err(BackendError::new(
                    "invalid_code_source",
                    "Snippet name must contain between 1 and 200 bytes",
                ));
            }
            let language = request.language.clone();
            if language.as_ref().is_some_and(|value| value.len() > 32) {
                return Err(BackendError::new(
                    "invalid_code_source",
                    "Snippet language identifier is too long",
                ));
            }
            Ok(ayla_code_analysis::CodeSource::Snippet {
                code,
                language,
                name,
            })
        }
        "git" => Err(BackendError::new(
            "remote_code_source_disabled",
            "Remote Git analysis is disabled in this integration",
        )),
        _ => Err(BackendError::new(
            "invalid_code_source",
            "Code-analysis source type must be 'directory' or 'snippet'",
        )),
    }
}

fn validate_code_analysis_parent(
    request: &CodeAnalysisRequest,
    parent: &crate::artifacts::CodeAnalysisRunRecord,
) -> Result<(), BackendError> {
    let matches = match request.source_type.as_str() {
        "directory" => request
            .directory_path
            .as_deref()
            .and_then(|path| std::fs::canonicalize(path).ok())
            .zip(
                parent
                    .directory_path
                    .as_deref()
                    .and_then(|path| std::fs::canonicalize(path).ok()),
            )
            .is_some_and(|(current, previous)| current == previous),
        "snippet" => {
            let name = request.name.as_deref().unwrap_or("snippet.rs");
            let language = request.language.as_deref().unwrap_or("");
            name == parent.source_label && language == parent.language.as_deref().unwrap_or("")
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(BackendError::new(
            "code_analysis_parent_mismatch",
            "Re-analysis parent does not describe the same local source",
        ))
    }
}

fn code_analysis_error(error: ayla_code_analysis::GenerateError) -> BackendError {
    match error {
        ayla_code_analysis::GenerateError::Cancelled => {
            BackendError::new("code_analysis_cancelled", "Code analysis cancelled")
        }
        ayla_code_analysis::GenerateError::Limit(message) => {
            BackendError::new("code_analysis_limit", message)
        }
        other => BackendError::new("code_analysis_failed", other.to_string()),
    }
}

fn string_field(record: &Value, field: &str) -> String {
    record
        .get(field)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn validate_source_file(file_path: &str) -> Result<std::path::PathBuf, BackendError> {
    let validated =
        crate::tools::quick_extract::validate_user_path(file_path).map_err(BackendError::from)?;
    let metadata = std::fs::metadata(&validated).map_err(|error| {
        BackendError::new(
            "file_metadata",
            format!("Cannot inspect source file: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(BackendError::new(
            "invalid_source_file",
            "Source path is not a file",
        ));
    }
    if metadata.len() > MAX_SOURCE_FILE {
        return Err(BackendError::new(
            "source_file_too_large",
            format!(
                "Source file is too large ({} bytes, max {})",
                metadata.len(),
                MAX_SOURCE_FILE
            ),
        ));
    }
    Ok(validated)
}

fn emit_chat_delta(line: &[u8], stream: &dyn StreamSink) -> Result<(), BackendError> {
    let value: Value = serde_json::from_slice(line).map_err(|error| {
        BackendError::new("ollama_stream_json", format!("Ollama stream JSON: {error}"))
    })?;
    emit_chat_delta_value(&value, stream)
}

fn emit_chat_delta_value(value: &Value, stream: &dyn StreamSink) -> Result<(), BackendError> {
    // Reasoning-capable Ollama models (for example Nemotron) stream their
    // internal working separately from the answer. Forward it as a distinct
    // event so hosts can make it visible without treating it as answer text.
    let thinking = value
        .get("message")
        .and_then(|message| message.get("thinking"))
        .or_else(|| value.get("thinking"))
        .and_then(Value::as_str);
    if let Some(thinking) = thinking {
        if !thinking.is_empty() {
            stream
                .send(serde_json::json!({ "type": "thinking", "content": thinking }))
                .map_err(chat_stream_error)?;
        }
    }

    if let Some(content) = value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
    {
        if !content.is_empty() {
            stream
                .send(serde_json::json!({ "type": "delta", "content": content }))
                .map_err(chat_stream_error)?;
        }
    }
    Ok(())
}

fn chat_stream_error(message: String) -> BackendError {
    BackendError::new("stream_error", format!("Chat channel: {message}"))
}

fn validate_session_payload_size(size: usize) -> Result<(), BackendError> {
    if size > MAX_MESSAGES_PAYLOAD {
        return Err(BackendError::new(
            "payload_too_large",
            format!(
                "Session payload too large ({} bytes, max {})",
                size, MAX_MESSAGES_PAYLOAD
            ),
        ));
    }
    Ok(())
}

fn validate_search_document_input(method: &str, size: usize) -> Result<(), BackendError> {
    if size > MAX_TEXT_PAYLOAD {
        return Err(BackendError::new(
            "payload_too_large",
            format!(
                "Processed document is too large ({} bytes, max {})",
                size, MAX_TEXT_PAYLOAD
            ),
        ));
    }
    if !matches!(method, "quick_extract" | "ocr") {
        return Err(BackendError::new(
            "unsupported_method",
            "Unsupported document processing method",
        ));
    }
    Ok(())
}

fn validate_file_payload_size(size: usize) -> Result<(), BackendError> {
    if size > MAX_TEXT_PAYLOAD {
        return Err(BackendError::new(
            "payload_too_large",
            format!(
                "File text too large ({} bytes, max {})",
                size, MAX_TEXT_PAYLOAD
            ),
        ));
    }
    Ok(())
}

fn validate_ingest_payload_size(size: usize) -> Result<(), BackendError> {
    if size > MAX_TEXT_PAYLOAD {
        return Err(BackendError::new(
            "payload_too_large",
            format!(
                "Payload too large ({} bytes, max {})",
                size, MAX_TEXT_PAYLOAD
            ),
        ));
    }
    Ok(())
}

fn value_field(record: &Value, field: &str) -> Value {
    record.get(field).cloned().unwrap_or_default()
}

fn build_search_history(
    sessions: &[Value],
    documents: &[Value],
    ingested_knowledge_paths: &HashSet<String>,
) -> Vec<SearchHistoryRecord> {
    sessions
        .iter()
        .filter_map(|session| {
            let search_id = string_field(session, "search_id");
            if search_id.is_empty() {
                return None;
            }
            let knowledge_ingested = session
                .get("knowledge_ingested")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || ingested_knowledge_paths.contains(&format!("search://{search_id}/knowledge.md"));
            let session_documents = documents
                .iter()
                .filter(|document| {
                    document.get("search_id").and_then(Value::as_str) == Some(search_id.as_str())
                })
                .cloned()
                .collect();
            Some(SearchHistoryRecord {
                status: session
                    .get("status")
                    .cloned()
                    .unwrap_or_else(|| Value::String("complete".into())),
                warnings: session
                    .get("warnings")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
                model: value_field(session, "model"),
                source_outcomes: session
                    .get("source_outcomes")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
                id: search_id.clone(),
                search_id,
                query: value_field(session, "query"),
                mode: session
                    .get("mode")
                    .cloned()
                    .unwrap_or_else(|| Value::String("knowledge".into())),
                timestamp: value_field(session, "created_at"),
                updated_at: value_field(session, "updated_at"),
                knowledge_document: value_field(session, "knowledge_document"),
                knowledge_ingested,
                synthesis: value_field(session, "synthesis"),
                papers: session
                    .get("papers")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
                documents: session_documents,
                total_results_found: value_field(session, "total_results_found"),
                pages_rendered: value_field(session, "pages_rendered"),
                persisted: true,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingStream {
        messages: std::sync::Mutex<Vec<Value>>,
    }

    impl StreamSink for RecordingStream {
        fn send(&self, payload: Value) -> Result<(), String> {
            self.messages.lock().unwrap().push(payload);
            Ok(())
        }
    }

    #[test]
    fn dto_serialization_preserves_frontend_field_names() {
        let data = KnowledgeBaseData {
            nodes: vec![KnowledgeNode {
                name: "ayla".into(),
                node_type: "Person".into(),
                source: "profile.md".into(),
                sources: vec!["profile.md".into()],
                aliases: vec!["ayla coworker assistant".into()],
                meaning: "The assistant persona.".into(),
                created: "2026-07-30T00:00:00Z".into(),
            }],
            edges: vec![],
            files: vec![],
        };
        let value = serde_json::to_value(data).unwrap();
        assert_eq!(value["nodes"][0]["type"], "Person");
        assert!(value["nodes"][0].get("node_type").is_none());
        assert_eq!(value["files"], serde_json::json!([]));
    }

    #[test]
    fn search_history_reconciles_committed_knowledge_and_groups_documents() {
        let sessions = vec![
            serde_json::json!({
                "search_id": "search-1",
                "query": "reinforcement learning",
                "created_at": "2026-07-30T10:00:00Z",
                "knowledge_ingested": false
            }),
            serde_json::json!({"query": "invalid session"}),
        ];
        let documents = vec![
            serde_json::json!({"search_id": "search-1", "document_id": "paper-1"}),
            serde_json::json!({"search_id": "other", "document_id": "paper-2"}),
        ];
        let paths = HashSet::from(["search://search-1/knowledge.md".to_string()]);
        let history = build_search_history(&sessions, &documents, &paths);

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].mode, serde_json::json!("knowledge"));
        assert!(history[0].knowledge_ingested);
        assert_eq!(history[0].documents.len(), 1);
        assert_eq!(history[0].documents[0]["document_id"], "paper-1");
    }

    #[test]
    fn group_a_dtos_preserve_existing_json_contracts() {
        let chat = ChatSessionRecord {
            id: "session-1".into(),
            title: "Research".into(),
            messages: vec![ChatMessageRecord {
                role: "user".into(),
                content: "Hello".into(),
            }],
        };
        let search = DocumentSearchResult {
            text: "A relevant chunk".into(),
            score: 0.75,
        };

        assert_eq!(
            serde_json::to_value(chat).unwrap()["messages"][0]["role"],
            "user"
        );
        assert_eq!(serde_json::to_value(search).unwrap()["score"], 0.75);
    }

    #[test]
    fn mutation_validation_enforces_transport_independent_limits() {
        let session_error = validate_session_payload_size(MAX_MESSAGES_PAYLOAD + 1).unwrap_err();
        assert_eq!(session_error.code, "payload_too_large");
        assert!(session_error
            .message
            .starts_with("Session payload too large"));

        let method_error = validate_search_document_input("shell", 10).unwrap_err();
        assert_eq!(method_error.code, "unsupported_method");
        assert_eq!(
            method_error.message,
            "Unsupported document processing method"
        );

        assert!(validate_search_document_input("quick_extract", MAX_TEXT_PAYLOAD).is_ok());
        assert!(validate_search_document_input("ocr", MAX_TEXT_PAYLOAD).is_ok());
        assert!(validate_file_payload_size(MAX_TEXT_PAYLOAD + 1).is_err());
        assert!(validate_ingest_payload_size(MAX_TEXT_PAYLOAD).is_ok());
        assert!(validate_ingest_payload_size(MAX_TEXT_PAYLOAD + 1).is_err());
    }

    #[test]
    fn group_c_dtos_preserve_existing_json_contracts() {
        let message = ProcessMessageResult {
            system_prompt: "memory + prompt".into(),
            tool_status: Some("tool completed".into()),
        };
        let consolidation = ConsolidationResult {
            merges: 2,
            pruned: 3,
            reinforced: 4,
        };
        let cleanup = GraphCleanupResult {
            proposed_groups: 1,
            proposed_node_refinements: 2,
            proposed_relation_refinements: 1,
            merged_groups: 1,
            merged_nodes: 2,
            renamed_nodes: 1,
            updated_meanings: 2,
            updated_node_types: 1,
            relabeled_relations: 1,
            deduplicated_edges: 3,
            skipped_groups: 0,
            groups: vec![GraphCleanupGroup {
                canonical: "markov decision process".into(),
                aliases: vec!["mdp".into(), "markov decision process".into()],
                confidence: 0.98,
                reason: "Same concept".into(),
            }],
            node_refinements: vec![GraphNodeRefinement {
                previous_name: "mdp model".into(),
                canonical_name: "markov decision process".into(),
                meaning: "A mathematical model for sequential decisions under uncertainty.".into(),
                confidence: 0.97,
                reason: "Expanded an unclear label".into(),
            }],
            relation_refinements: vec![GraphRelationRefinement {
                from: "agent".into(),
                to: "environment".into(),
                previous_relation: "interacts with the".into(),
                improved_relation: "INTERACTS_WITH".into(),
                confidence: 0.98,
                reason: "Canonical predicate".into(),
            }],
        };

        let message = serde_json::to_value(message).unwrap();
        assert_eq!(message["system_prompt"], "memory + prompt");
        assert_eq!(message["tool_status"], "tool completed");

        let consolidation = serde_json::to_value(consolidation).unwrap();
        assert_eq!(
            consolidation,
            serde_json::json!({"merges": 2, "pruned": 3, "reinforced": 4})
        );

        let cleanup = serde_json::to_value(cleanup).unwrap();
        assert_eq!(cleanup["merged_groups"], 1);
        assert_eq!(cleanup["groups"][0]["canonical"], "markov decision process");
        let confidence = cleanup["groups"][0]["confidence"].as_f64().unwrap();
        assert!((confidence - 0.98).abs() < 0.0001);
    }

    #[test]
    fn chat_stream_parser_emits_thinking_and_nonempty_deltas() {
        let stream = RecordingStream::default();
        emit_chat_delta(
            br#"{"message":{"thinking":"checking the problem"},"done":false}"#,
            &stream,
        )
        .unwrap();
        emit_chat_delta(br#"{"message":{"content":"hello"},"done":false}"#, &stream).unwrap();
        emit_chat_delta(br#"{"message":{"content":""},"done":false}"#, &stream).unwrap();
        emit_chat_delta(br#"{"done":true}"#, &stream).unwrap();

        let messages = stream.messages.lock().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0],
            serde_json::json!({"type": "thinking", "content": "checking the problem"})
        );
        assert_eq!(
            messages[1],
            serde_json::json!({"type": "delta", "content": "hello"})
        );
        drop(messages);

        let error = emit_chat_delta(b"not json", &stream).unwrap_err();
        assert_eq!(error.code, "ollama_stream_json");
    }
}
