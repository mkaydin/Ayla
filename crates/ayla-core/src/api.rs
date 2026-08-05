use crate::db::surreal::SurrealDB;
use crate::memory::graph::NodeType;
use crate::memory::graph_cleanup::{CleanupEdge, CleanupNode, GraphCleanupAnalyzer};
use crate::memory::MemoryManager;
use crate::processes::ProcessRegistry;
use crate::tools::types::{IngestionResult, SourceInfo};
use crate::{BackendError, EventSink, ResourceLocator, StreamSink};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const MAX_TEXT_PAYLOAD: usize = 50 * 1024 * 1024;
const MAX_MESSAGES_PAYLOAD: usize = 10 * 1024 * 1024;
const MAX_SOURCE_FILE: u64 = 100 * 1024 * 1024;
const CHAT_MAX_OUTPUT_TOKENS: usize = 4_096;

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

/// First host-neutral command surface shared by Tauri and the Electron sidecar.
///
/// Operations are migrated here incrementally. Adapters must preserve their
/// existing public command names and JSON shapes while delegating to this API.
pub struct BackendApi<'manager> {
    manager: &'manager MemoryManager,
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
        let rows = db
            .query("SELECT * FROM kb_file ORDER BY date DESC;")
            .await
            .map_err(BackendError::from)?;
        log::info!("Files: found {} records", rows.len());

        Ok(rows
            .iter()
            .map(|record| FileRecord {
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
        self.manager.get_document_text(name).await
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
                self.manager
                    .store_document_text(source, result.full_text.clone())
                    .await;
                let mut ingestion = pipeline.lock().await;
                let message = ingestion
                    .ingest(result, Some(events))
                    .await
                    .map_err(BackendError::from)?;
                {
                    let mut graph = self.manager.graph.lock().await;
                    let _ = graph.sync().await;
                }
                crate::memory::save_text_store(self.manager).await;
                log::info!("Ingest: {}", message);
                Ok(message)
            }
            None => {
                log::info!("Ingest: DB offline, storing in graph and text store");
                let mut graph = self.manager.graph.lock().await;
                graph.upsert_node(&source, NodeType::Document);
                let text_store_len = self
                    .manager
                    .store_document_text(source, result.full_text.clone())
                    .await;
                let node_count = graph.node_count();
                let edge_count = graph.edge_count();
                log::info!(
                    "Ingest: graph={} nodes/{} edges, text_store={} docs",
                    node_count,
                    edge_count,
                    text_store_len
                );
                Ok(format!(
                    "Stored {} chars in memory. Graph: {} nodes, {} edges.",
                    result.full_text.len(),
                    node_count,
                    edge_count
                ))
            }
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
        _resources: &ResourceLocator,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<OcrResult, BackendError> {
        let validated = validate_source_file(file_path)?;
        let file_path = validated.to_string_lossy().to_string();
        let pid = uuid::Uuid::new_v4().to_string();
        log::info!("OCR [{}]: starting for {}", pid, file_path);

        let options = ayla_ocr::OcrOptions {
            model: self.manager.config.ocr_model.clone(),
            ollama_url: self.manager.config.ollama_url.clone(),
            request_timeout_secs: self.manager.config.ollama_long_timeout_secs as u64,
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
        log::info!("OCR [{}]: done — {} chars", pid, document.text.len());
        Ok(OcrResult {
            chars: document.text.len(),
            text: document.text,
            pages: document.pages_processed.to_string(),
            pid,
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
                "SELECT search_id, query, mode, created_at, updated_at, knowledge_document, knowledge_ingested, synthesis, papers, total_results_found, pages_rendered FROM search_session ORDER BY created_at DESC LIMIT 50;",
            )
            .await
            .map_err(BackendError::from)?;
        let documents = db
            .query(
                "SELECT document_id, search_id, name, path, source_url, mime_type, size_bytes, status, extraction_method, chars, chunks, processed_at, ingested_at, created_at FROM search_document ORDER BY created_at ASC;",
            )
            .await
            .map_err(BackendError::from)?;
        let ingested_knowledge_paths: HashSet<String> = db
            .query("SELECT path FROM kb_file WHERE source = 'meta_search_knowledge';")
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
        variables.insert("now".into(), Value::String(chrono::Utc::now().to_rfc3339()));
        db.exec_with_vars(
            "UPDATE search_document SET status = 'processed', extraction_method = $method, extracted_text = $text, chars = $chars, chunks = $chunks, processed_at = $now, updated_at = $now WHERE search_id = $search_id AND document_id = $document_id RETURN NONE;",
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
        self.manager.store_document_text(name.clone(), text).await;
        log::info!("File record: saved successfully");
        Ok(())
    }

    pub async fn delete_file_record(
        &self,
        name: &str,
        delete_nodes: bool,
    ) -> Result<String, BackendError> {
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        let db = self.connect_database().await?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("name".into(), Value::String(name.to_string()));
        db.exec_with_vars("DELETE kb_file WHERE name = $name;", variables)
            .await
            .map_err(BackendError::from)?;
        self.manager.remove_document_text(name).await;

        let mut removed_nodes = 0;
        if delete_nodes {
            let mut graph = self.manager.graph.lock().await;
            removed_nodes = graph.remove_by_source(name);
            graph.sync().await.map_err(BackendError::from)?;
        }
        if let Some(document_store) = &self.manager.document_store {
            if let Err(error) = document_store.delete_by_filename(name).await {
                log::warn!("Failed to delete vectors for '{}': {}", name, error);
            }
        }
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
