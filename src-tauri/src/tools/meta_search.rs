// ── Meta Search tool: subprocess wrapper for meta_search ───

use super::types::*;
use super::IngestTool;
use std::path::PathBuf;
use tokio::process::Command;

pub struct MetaSearchTool {
    bin_path: PathBuf,
}

impl MetaSearchTool {
    pub fn new() -> Self {
        Self { bin_path: Self::find_binary() }
    }

    fn find_binary() -> PathBuf {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let external = base.join("../../meta_search_Js_render/target/release/meta_search");
        if external.exists() { return external.canonicalize().unwrap_or(external); }
        PathBuf::from("meta_search")
    }
}

#[async_trait::async_trait]
impl IngestTool for MetaSearchTool {
    fn name(&self) -> &str { "search" }

    fn description(&self) -> &str {
        "Searches scientific literature across multiple engines, downloads and analyzes papers."
    }

    async fn is_available(&self) -> bool {
        let ok = self.bin_path.exists();
        if !ok {
            log::warn!("Meta Search tool not found at {:?}. Build it: cd ../meta_search_Js_render && cargo build --release", self.bin_path);
        }
        ok
    }

    async fn run(&self, query: &str, _file_path: Option<&str>) -> Result<IngestionResult, String> {
        let output = Command::new(&self.bin_path)
            .args([
                "-j", "-n", "10",
                "--model", "gemma4:e4b",
                "--searxng-url", "http://127.0.0.1:8888",
                "--lightpanda-url", "ws://127.0.0.1:9222",
                query,
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to run meta_search: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("meta_search failed: {}", stderr.lines().last().unwrap_or("unknown error")));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        // Strip banner output (everything before the first '{')
        let trimmed = stdout.trim();
        let json_start = trimmed.find('{').unwrap_or(0);
        let json_str = &trimmed[json_start..];

        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| format!("Failed to parse output after banner: {} (first 100 chars: {})",
                e, &json_str[..json_str.len().min(100)]))?;

        let synthesis = parsed["synthesis"]["summary"]
            .as_str()
            .unwrap_or("No synthesis available")
            .to_string();

        let mut full_text = format!("# Research Query: {}\n\n## Synthesis\n{}\n\n", query, synthesis);

        let papers_key = if parsed.get("papers_analyzed").is_some() { "papers_analyzed" } else { "papers" };
        let entities: Vec<String> = if let Some(papers) = parsed[papers_key].as_array() {
            for paper in papers {
                let title = paper["title"].as_str().unwrap_or("Untitled");
                let findings = paper["key_findings"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("; "))
                    .unwrap_or_default();
                let score = paper["relevance_score"].as_f64().unwrap_or(0.0);
                full_text.push_str(&format!("## {} (score: {:.2})\nFindings: {}\n\n", title, score, findings));
            }
            papers.iter()
                .filter_map(|p| p["title"].as_str().map(String::from))
                .collect()
        } else {
            vec![]
        };

        Ok(IngestionResult {
            summary: format!("Searched '{}': {} papers.", query, entities.len()),
            entities,
            facts: vec![],
            relations: vec![],
            full_text,
            source: SourceInfo {
                tool: "meta_search".into(),
                input: query.into(),
                output_path: None,
                metadata: serde_json::json!({"query": query}),
            },
            timestamp: chrono::Utc::now(),
        })
    }
}
