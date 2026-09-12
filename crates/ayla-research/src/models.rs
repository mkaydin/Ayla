use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub raw: String,
    pub enriched: String,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: String,
    pub is_pdf: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadedPdf {
    pub original_url: String,
    pub file_path: PathBuf,
    pub sha256: String,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedPage {
    pub url: String,
    pub title: String,
    pub text_content: String,
    pub status_code: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedSource {
    pub source_url: String,
    pub source_type: SourceType,
    pub text: String,
    pub length_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    Pdf { file_path: PathBuf, sha256: String },
    WebPage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperAnalysis {
    pub source_url: String,
    pub title: String,
    pub authors: String,
    pub year: Option<u32>,
    pub abstract_text: String,
    pub key_findings: Vec<String>,
    pub methodology: String,
    pub datasets: Vec<String>,
    pub relevance_score: f64,
    pub relevance_rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisReport {
    #[serde(default)]
    pub query: String,
    pub cross_cutting_themes: Vec<String>,
    pub consensus_findings: Vec<String>,
    #[serde(default)]
    pub conflicting_findings: Vec<String>,
    #[serde(default)]
    pub gaps_identified: Vec<String>,
    #[serde(default)]
    pub ranked_papers: Vec<PaperAnalysis>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionReport {
    #[serde(default)]
    pub downloaded_sources: Vec<DownloadedPdf>,
    #[serde(default)]
    pub source_outcomes: Vec<serde_json::Value>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub session_id: Uuid,
    pub query: String,
    pub enriched_query: String,
    pub total_results_found: usize,
    pub pdfs_downloaded: usize,
    pub pages_rendered: usize,
    pub papers_analyzed: Vec<PaperAnalysis>,
    pub synthesis: SynthesisReport,
    pub download_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaAnalyzeRequest {
    pub model: String,
    pub prompt: String,
    pub system: String,
    pub stream: bool,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaAnalyzeResponse {
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearxngResponse {
    #[serde(default)]
    pub unresponsive_engines: Vec<serde_json::Value>,
    #[serde(default)]
    pub query: String,
    pub results: Vec<SearxngResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearxngResult {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub engine: String,
    pub score: Option<f64>,
    pub category: Option<String>,
}
