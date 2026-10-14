mod types;
pub use types::*;
mod chat;
mod cleanup;
mod code_analysis;
mod documents;
mod graph;
mod history;
mod ocr;

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

impl<'manager> BackendApi<'manager> {
    pub fn new(manager: &'manager MemoryManager) -> Self {
        Self { manager }
    }

    pub fn check_db_ready(&self) -> bool {
        self.manager.is_db_ready()
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
