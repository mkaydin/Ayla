// ── Scientific research tool ───

use super::types::*;
use super::IngestTool;

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

pub struct MetaSearchTool;

impl MetaSearchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl IngestTool for MetaSearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Searches, downloads, and analyzes scientific literature with Ayla's in-process research engine."
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn run(&self, query: &str, _file_path: Option<&str>) -> Result<IngestionResult, String> {
        let config = crate::MemoryConfig::default();
        let data_dir = std::env::var_os("AYLA_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join("ayla-data")
            })
            .join("research");
        let report = ayla_research::search(
            query,
            ayla_research::ResearchOptions {
                searxng_url: config.searxng_url,
                lightpanda_url: config.lightpanda_url,
                ollama_url: config.ollama_url,
                model: config.llm_model,
                max_num_ctx: config.ollama_num_ctx,
                request_timeout_secs: config.ollama_long_timeout_secs as u64,
                max_papers: 10,
                search_pages: 2,
                data_dir,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        let parsed = serde_json::to_value(report).map_err(|error| error.to_string())?;

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
                tool: "ayla_research".into(),
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
    use super::build_knowledge_document;

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
