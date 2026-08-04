// ── Meta Search tool: subprocess wrapper for meta_search ───

use super::types::*;
use super::IngestTool;
use std::path::PathBuf;
use tokio::process::Command;

/// Extract the final JSON object from `meta_search` stdout.
///
/// The external binary prints banners and tracing output before its JSON report. Some
/// Lightpanda log messages contain their own JSON objects, so selecting the first `{`
/// parses a CDP error instead of the report. Walking object starts from the end ensures
/// we select the complete, final JSON document.
pub fn parse_meta_search_stdout(stdout: &str) -> Result<serde_json::Value, String> {
    let trimmed = stdout.trim();
    let mut last_error = None;

    for (index, character) in trimmed.char_indices().rev() {
        if character != '{' {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(trimmed[index..].trim()) {
            Ok(value) if value.is_object() => return Ok(value),
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
    }

    let detail = last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "no JSON object found".to_string());
    Err(format!(
        "Meta Search returned no valid final JSON report: {} (output tail: {})",
        detail,
        crate::memory::safe_truncate(
            trimmed
                .get(trimmed.len().saturating_sub(240)..)
                .unwrap_or(trimmed),
            240,
        )
    ))
}

fn report_array<'a>(report: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    report
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn report_string<'a>(report: &'a serde_json::Value, key: &str) -> &'a str {
    report
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn inferred_title(paper: &serde_json::Value) -> String {
    let title = report_string(paper, "title").trim();
    if !title.is_empty() {
        return title.to_string();
    }
    report_string(paper, "abstract_text")
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Untitled source")
        .trim()
        .to_string()
}

/// Convert a search report into a clean Markdown document designed for the KB ingestion
/// pipeline. It retains provenance and evidence rather than serializing UI state as JSON.
pub fn build_knowledge_document(query: &str, report: &serde_json::Value) -> String {
    let synthesis = report.get("synthesis").unwrap_or(&serde_json::Value::Null);
    let summary = report_string(synthesis, "summary");
    let papers = report
        .get("papers_analyzed")
        .or_else(|| report.get("papers"))
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let mut document = format!(
        "# General Knowledge: {query}\n\n\
         > Generated from a multi-source search. Source URLs and evidence are retained for provenance.\n\n\
         ## Overview\n\n{}\n\n",
        if summary.trim().is_empty() {
            "No standalone synthesis was returned. The source evidence is summarized below."
        } else {
            summary.trim()
        }
    );

    let sections = [
        ("Core Themes", "cross_cutting_themes"),
        ("Consensus Findings", "consensus_findings"),
        ("Conflicting Findings", "conflicting_findings"),
        ("Knowledge Gaps", "gaps_identified"),
    ];
    for (heading, key) in sections {
        let values = report_array(synthesis, key);
        if values.is_empty() {
            continue;
        }
        document.push_str(&format!("## {heading}\n\n"));
        for value in values.iter().filter_map(serde_json::Value::as_str) {
            document.push_str(&format!("- {}\n", value.trim()));
        }
        document.push('\n');
    }

    if !papers.is_empty() {
        document.push_str("## Source Evidence\n\n");
    }
    for (index, paper) in papers.iter().enumerate() {
        let title = inferred_title(paper);
        document.push_str(&format!("### {}. {}\n\n", index + 1, title));

        let authors = report_string(paper, "authors");
        let year = paper
            .get("year")
            .and_then(|year| {
                year.as_u64()
                    .map(|number| number.to_string())
                    .or_else(|| year.as_str().map(ToOwned::to_owned))
            })
            .unwrap_or_default();
        let url = report_string(paper, "source_url");
        if !authors.trim().is_empty() {
            document.push_str(&format!("- Authors: {}\n", authors.trim()));
        }
        if !year.trim().is_empty() {
            document.push_str(&format!("- Year: {}\n", year.trim()));
        }
        if !url.trim().is_empty() {
            document.push_str(&format!("- Source: {}\n", url.trim()));
        }
        if let Some(score) = paper
            .get("relevance_score")
            .and_then(|value| value.as_f64())
        {
            document.push_str(&format!("- Relevance: {:.2}\n", score));
        }
        document.push('\n');

        let findings = report_array(paper, "key_findings");
        if !findings.is_empty() {
            document.push_str("Key findings:\n\n");
            for finding in findings.iter().filter_map(serde_json::Value::as_str) {
                document.push_str(&format!("- {}\n", finding.trim()));
            }
            document.push('\n');
        }

        let methodology = report_string(paper, "methodology");
        if !methodology.trim().is_empty() {
            document.push_str(&format!("Methodology: {}\n\n", methodology.trim()));
        }
        let abstract_text = report_string(paper, "abstract_text");
        if !abstract_text.trim().is_empty() {
            document.push_str(&format!(
                "Evidence summary: {}\n\n",
                crate::memory::safe_truncate(abstract_text.trim(), 2_000)
            ));
        }
    }

    document
}

pub struct MetaSearchTool {
    bin_path: PathBuf,
}

impl MetaSearchTool {
    pub fn new() -> Self {
        Self {
            bin_path: Self::find_binary(),
        }
    }

    fn find_binary() -> PathBuf {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let external = base.join("../../../meta_search_Js_render/target/release/meta_search");
        if external.exists() {
            return external.canonicalize().unwrap_or(external);
        }
        PathBuf::from("meta_search")
    }
}

#[async_trait::async_trait]
impl IngestTool for MetaSearchTool {
    fn name(&self) -> &str {
        "search"
    }

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
        let model = std::env::var("ALICE_LLM_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "gemma4:e4b".to_string());
        let output = Command::new(&self.bin_path)
            .args([
                "-j",
                "-n",
                "10",
                "--model",
                &model,
                "--searxng-url",
                "http://127.0.0.1:8888",
                "--lightpanda-url",
                "ws://127.0.0.1:9222",
                query,
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to run meta_search: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "meta_search failed: {}",
                stderr.lines().last().unwrap_or("unknown error")
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let parsed = parse_meta_search_stdout(&stdout)?;

        let mut full_text = build_knowledge_document(query, &parsed);

        let papers_key = if parsed.get("papers_analyzed").is_some() {
            "papers_analyzed"
        } else {
            "papers"
        };
        let entities: Vec<String> = if let Some(papers) = parsed[papers_key].as_array() {
            for paper in papers {
                let title = paper["title"].as_str().unwrap_or("Untitled");
                let findings = paper["key_findings"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .unwrap_or_default();
                let score = paper["relevance_score"].as_f64().unwrap_or(0.0);
                if !findings.is_empty() {
                    full_text.push_str(&format!(
                        "\n## Extracted Findings: {} (score: {:.2})\n{}\n",
                        title, score, findings
                    ));
                }
            }
            papers
                .iter()
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

#[cfg(test)]
mod tests {
    use super::{build_knowledge_document, parse_meta_search_stdout};

    #[test]
    fn parses_final_report_after_json_shaped_log_messages() {
        let output = concat!(
            "Meta Search banner\n",
            "WARN CDP error: {\"code\":-31998,\"message\":\"BrowserContextNotLoaded\"}\n",
            "{\"session_id\":\"session-1\",\"query\":\"graph theory\",",
            "\"papers_analyzed\":[],\"synthesis\":{\"summary\":\"Connected concepts\"}}\n"
        );

        let parsed = parse_meta_search_stdout(output).expect("final report should parse");
        assert_eq!(parsed["session_id"], "session-1");
        assert_eq!(parsed["synthesis"]["summary"], "Connected concepts");
    }

    #[test]
    #[ignore = "requires ALICE_META_SEARCH_FIXTURE pointing to captured Meta Search stdout"]
    fn parses_captured_meta_search_output() {
        let path = std::env::var("ALICE_META_SEARCH_FIXTURE")
            .expect("set ALICE_META_SEARCH_FIXTURE to a captured stdout file");
        let output = std::fs::read_to_string(path).expect("read Meta Search fixture");
        let parsed = parse_meta_search_stdout(&output).expect("captured report should parse");
        assert!(parsed.get("session_id").is_some());
        assert!(parsed
            .get("papers_analyzed")
            .and_then(serde_json::Value::as_array)
            .is_some());
    }

    #[test]
    fn knowledge_document_retains_structure_and_provenance() {
        let report = serde_json::json!({
            "synthesis": {
                "summary": "A concise overview.",
                "cross_cutting_themes": ["Theme A"],
                "consensus_findings": ["Finding A"]
            },
            "papers_analyzed": [{
                "title": "Paper A",
                "authors": "A. Author",
                "year": 2025,
                "source_url": "https://example.test/paper",
                "key_findings": ["Evidence A"],
                "abstract_text": "Abstract evidence."
            }]
        });

        let document = build_knowledge_document("Graph systems", &report);
        assert!(document.contains("# General Knowledge: Graph systems"));
        assert!(document.contains("## Core Themes"));
        assert!(document.contains("https://example.test/paper"));
        assert!(document.contains("Evidence A"));
    }
}
