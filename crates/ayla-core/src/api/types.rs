//! Public backend request and response contracts.

use super::*;

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
