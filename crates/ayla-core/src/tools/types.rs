// ── Shared tool types ──────────────────────────────────────

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionResult {
    pub summary: String,
    pub entities: Vec<String>,
    pub facts: Vec<Fact>,
    pub relations: Vec<Relation>,
    pub full_text: String,
    pub source: SourceInfo,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub from: String,
    pub relation_type: String,
    pub to: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub tool: String,
    pub input: String,
    pub output_path: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct ToolDispatch {
    pub tool: String,
    pub query: String,
    pub file_path: Option<String>,
}
