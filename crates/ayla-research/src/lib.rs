pub mod ai;
pub mod downloader;
pub mod extractor;
pub mod job;
pub mod models;
pub mod orchestrator;
pub mod public_fetch;
pub mod renderer;
pub mod searcher;
pub mod synthesizer;

use ai::AiClient;
use anyhow::{Context, Result};
use downloader::Downloader;
use models::SessionReport;
use orchestrator::Orchestrator;
use renderer::Renderer;
use searcher::Searcher;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ResearchOptions {
    pub searxng_url: String,
    pub lightpanda_url: String,
    pub ollama_url: String,
    pub model: String,
    pub max_num_ctx: usize,
    pub request_timeout_secs: u64,
    pub max_papers: usize,
    pub search_pages: u32,
    pub data_dir: PathBuf,
}

impl Default for ResearchOptions {
    fn default() -> Self {
        Self {
            searxng_url: "http://127.0.0.1:8888".into(),
            lightpanda_url: "ws://127.0.0.1:9222".into(),
            ollama_url: "http://127.0.0.1:11434".into(),
            model: "gemma4:e4b".into(),
            max_num_ctx: 32_768,
            request_timeout_secs: 0,
            max_papers: 10,
            search_pages: 2,
            data_dir: std::env::temp_dir().join("ayla-research"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

pub type ProgressCallback<'a> = dyn Fn(ResearchProgress) + Send + Sync + 'a;

pub(crate) fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Run the complete scientific paper pipeline in the current Rust process.
pub async fn search(query: &str, options: ResearchOptions) -> Result<SessionReport> {
    search_with_progress(query, options, None).await
}

pub async fn search_with_progress(
    query: &str,
    options: ResearchOptions,
    progress: Option<&ProgressCallback<'_>>,
) -> Result<SessionReport> {
    if query.trim().is_empty() {
        anyhow::bail!("Research query cannot be empty");
    }
    let searcher = Searcher::with_base_url(options.searxng_url)
        .context("Failed to initialize SearXNG search client")?;
    let downloader = Downloader::new().context("Failed to initialize PDF downloader")?;
    let renderer = Renderer::new()
        .context("Failed to initialize page renderer")?
        .with_ws_url(options.lightpanda_url);
    let ai = AiClient::new()?
        .with_base_url(&options.ollama_url)
        .with_model(&options.model)
        .with_context_limit(options.max_num_ctx)
        .with_request_timeout(options.request_timeout_secs)?;
    let orchestrator =
        Orchestrator::with_data_dir(searcher, downloader, renderer, ai, options.data_dir)?;
    orchestrator
        .run_with_progress(query, options.max_papers, options.search_pages, progress)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_truncation_never_splits_a_character() {
        assert_eq!(truncate_utf8("Ayla 📚 research", 8), "Ayla ");
        assert_eq!(truncate_utf8("short", 50), "short");
    }

    #[test]
    fn local_models_are_sequential_and_cloud_models_are_bounded() {
        let local = AiClient::new()
            .expect("local client")
            .with_model("gemma4:e4b");
        let cloud = AiClient::new()
            .expect("cloud client")
            .with_model("nemotron-3-super:cloud");
        assert_eq!(local.recommended_concurrency(), 1);
        assert_eq!(cloud.recommended_concurrency(), 2);
    }
}
