use crate::models::{OllamaAnalyzeResponse, PaperAnalysis, SynthesisReport};
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::json;
use tracing::{debug, warn};

const OLLAMA_BASE: &str = "http://localhost:11434";
const MODEL: &str = "gemma4:e4b";
const CONTEXT_BUCKETS: [usize; 5] = [8_192, 16_384, 32_768, 65_536, 131_072];

#[derive(Debug, Clone)]
pub struct AiClient {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_num_ctx: usize,
}

impl AiClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url: OLLAMA_BASE.to_string(),
            model: MODEL.to_string(),
            max_num_ctx: 32_768,
        })
    }

    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn recommended_concurrency(&self) -> usize {
        if self.model.to_ascii_lowercase().contains(":cloud") {
            2
        } else {
            1
        }
    }

    pub fn with_context_limit(mut self, max_num_ctx: usize) -> Self {
        self.max_num_ctx = max_num_ctx.max(8_192);
        self
    }

    pub fn with_request_timeout(mut self, timeout_secs: u64) -> Result<Self> {
        let builder =
            reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15));
        let builder = if timeout_secs == 0 {
            builder
        } else {
            builder.timeout(std::time::Duration::from_secs(timeout_secs))
        };
        self.client = builder
            .build()
            .context("Failed to build Ollama HTTP client")?;
        Ok(self)
    }

    fn context_for(&self, input_bytes: usize, output_tokens: usize) -> usize {
        let input_tokens = input_bytes.div_ceil(3);
        let required = input_tokens
            .saturating_add(output_tokens)
            .saturating_add((input_tokens / 4).max(512));
        CONTEXT_BUCKETS
            .into_iter()
            .find(|bucket| *bucket >= required)
            .unwrap_or(self.max_num_ctx)
            .min(self.max_num_ctx)
            .max(8_192.min(self.max_num_ctx))
    }

    pub async fn enrich_query(&self, query: &str) -> Result<String> {
        let system = "You are a scientific research assistant. \
                      Expand the user's search query with relevant academic terms and synonyms. \
                      Do NOT include 'filetype:pdf' or site restrictions — the search engine handles that. \
                      Return ONLY the enriched search query string, no explanation, no quotes.";

        let prompt = format!(
            "Enrich this scientific search query for maximum coverage on academic search engines:\n\n{}",
            query
        );

        let response = self.chat(system, &prompt).await?;
        let cleaned = response.trim().trim_matches('"').trim().to_string();
        Ok(cleaned)
    }

    pub async fn analyze_paper(
        &self,
        source_url: &str,
        text: &str,
        query: &str,
    ) -> Result<PaperAnalysis> {
        let truncated = crate::truncate_utf8(text, 80_000);

        let system = r#"You extract structured metadata from scientific papers. 
Your entire response must be ONLY a raw JSON object — no markdown fences, no explanation, no code blocks.
The JSON must use exactly these keys (all present, none omitted):

{
  "title": "exact paper title here",
  "authors": "comma-separated author names",
  "year": 2024,
  "abstract_text": "the abstract paragraph",
  "key_findings": ["specific finding 1", "specific finding 2", "specific finding 3"],
  "methodology": "brief methodology description",
  "datasets": ["dataset name if mentioned"],
  "relevance_score": 0.85,
  "relevance_rationale": "one sentence why this relates to the query"
}

relevance_score must be 0.0 to 1.0. If the paper is completely unrelated, score 0.0.
Give 3-5 concrete key findings. Empty arrays are acceptable only if the data truly isn't present."#;

        let prompt = format!(
            "Analyze this paper for relevance to: \"{}\"\n\nPaper URL: {}\n\nContent:\n{}",
            query, source_url, truncated
        );

        let response = self.chat_json(system, &prompt).await?;
        let analysis = parse_paper_analysis(source_url, &response, truncated);

        if analysis.relevance_score > 0.0 && !analysis.title.is_empty() {
            return Ok(analysis);
        }

        warn!(
            "JSON parse failed, retrying without format enforcement for {}",
            source_url
        );
        let response2 = self.chat(system, &prompt).await?;
        let analysis2 = parse_paper_analysis(source_url, &response2, truncated);
        Ok(analysis2)
    }

    pub async fn synthesize(
        &self,
        query: &str,
        papers: &[PaperAnalysis],
    ) -> Result<SynthesisReport> {
        if papers.is_empty() {
            return Ok(SynthesisReport {
                query: query.to_string(),
                cross_cutting_themes: vec![],
                consensus_findings: vec![],
                conflicting_findings: vec![],
                gaps_identified: vec!["No papers could be analyzed.".to_string()],
                ranked_papers: vec![],
                summary: "No papers available for synthesis.".to_string(),
            });
        }

        let valid_papers: Vec<_> = papers
            .iter()
            .filter(|p| !p.title.is_empty() || !p.abstract_text.is_empty())
            .collect();

        if valid_papers.is_empty() {
            return Ok(build_fallback_synthesis(query, papers));
        }

        let papers_text: String = valid_papers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "Paper {}:\n  Title: {}\n  Authors: {}\n  Year: {}\n  Abstract: {}\n  Key Findings: {}\n  Methodology: {}\n  Datasets: {}\n  Score: {:.0}%\n",
                    i + 1,
                    p.title,
                    p.authors,
                    p.year.map_or("N/A".to_string(), |y| y.to_string()),
                    p.abstract_text.chars().take(600).collect::<String>(),
                    p.key_findings.join("; "),
                    p.methodology,
                    p.datasets.join(", "),
                    p.relevance_score * 100.0
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let system = r#"You synthesize scientific literature surveys.
Your entire response must be ONLY a raw JSON object — no markdown fences, no explanation.
The JSON must use exactly these keys:

{
  "cross_cutting_themes": ["theme that appears across multiple papers"],
  "consensus_findings": ["finding where papers generally agree"],
  "conflicting_findings": ["finding where papers disagree or differ in results"],
  "gaps_identified": ["research gap or under-explored area"],
  "summary": "comprehensive 3-4 paragraph summary of the state of the art"
}

Provide 2-4 items per array. Be specific and reference actual paper findings."#;

        let prompt = format!(
            "Synthesize these {} papers about: \"{}\"\n\n{}",
            valid_papers.len(),
            query,
            papers_text
        );

        let response = self.chat_json(system, &prompt).await?;
        let synth = parse_synthesis_report(query, &response, papers);

        if synth.summary.len() < 20 || synth.cross_cutting_themes.is_empty() {
            warn!("Synthesis parse weak, retrying without format enforcement");
            let response2 = self.chat(system, &prompt).await?;
            Ok(parse_synthesis_report(query, &response2, papers))
        } else {
            Ok(synth)
        }
    }

    pub async fn score_relevance_batch(
        &self,
        _query: &str,
        papers: &[PaperAnalysis],
    ) -> Result<Vec<PaperAnalysis>> {
        let mut scored = papers.to_vec();
        scored.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(scored)
    }

    async fn chat(&self, system: &str, prompt: &str) -> Result<String> {
        let num_ctx = self.context_for(system.len() + prompt.len(), 4096);
        let body = json!({
            "model": self.model,
            "system": system,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.3,
                "num_predict": 4096,
                "num_ctx": num_ctx
            }
        });

        let url = format!("{}/api/generate", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("Failed to reach Ollama")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read Ollama response")?;
        if !status.is_success() {
            anyhow::bail!(
                "Ollama HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 500)
            );
        }
        let result: OllamaAnalyzeResponse =
            serde_json::from_str(&body).context("Failed to parse Ollama response")?;

        Ok(result.response)
    }

    async fn chat_json(&self, system: &str, prompt: &str) -> Result<String> {
        let num_ctx = self.context_for(system.len() + prompt.len(), 8192);
        let body = json!({
            "model": self.model,
            "system": system,
            "prompt": prompt,
            "stream": false,
            "options": {
                "temperature": 0.1,
                "num_predict": 8192,
                "num_ctx": num_ctx
            },
            "format": "json"
        });

        let url = format!("{}/api/generate", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("Failed to reach Ollama")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read Ollama response")?;
        if !status.is_success() {
            anyhow::bail!(
                "Ollama HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 500)
            );
        }
        let result: OllamaAnalyzeResponse =
            serde_json::from_str(&body).context("Failed to parse Ollama response")?;

        Ok(result.response)
    }

    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

fn parse_paper_analysis(source_url: &str, raw: &str, fallback_text: &str) -> PaperAnalysis {
    let cleaned = clean_json_response(raw);

    let parsed: Option<PaperAnalysis> = serde_json::from_str(&cleaned)
        .ok()
        .or_else(|| extract_paper_json_regex(&cleaned));

    match parsed {
        Some(mut analysis) => {
            analysis.source_url = source_url.to_string();
            if analysis.relevance_score < 0.0 || analysis.relevance_score > 1.0 {
                analysis.relevance_score = 0.5;
            }
            if analysis.title.is_empty() {
                analysis.title = extract_field(&cleaned, "title").unwrap_or_default();
            }
            if analysis.abstract_text.is_empty() {
                analysis.abstract_text = fallback_text.chars().take(1200).collect();
            }
            analysis
        }
        None => {
            debug!(
                "All JSON parsing failed for {}. Using regex field extraction.",
                source_url
            );
            PaperAnalysis {
                source_url: source_url.to_string(),
                title: extract_field(&cleaned, "title").unwrap_or_default(),
                authors: extract_field(&cleaned, "authors").unwrap_or_default(),
                year: extract_field(&cleaned, "year").and_then(|s| s.parse().ok()),
                abstract_text: extract_field(&cleaned, "abstract_text")
                    .unwrap_or_else(|| fallback_text.chars().take(1200).collect()),
                key_findings: extract_array_field(&cleaned, "key_findings"),
                methodology: extract_field(&cleaned, "methodology").unwrap_or_default(),
                datasets: extract_array_field(&cleaned, "datasets"),
                relevance_score: extract_field(&cleaned, "relevance_score")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.5),
                relevance_rationale: extract_field(&cleaned, "relevance_rationale")
                    .unwrap_or_default(),
            }
        }
    }
}

fn parse_synthesis_report(query: &str, raw: &str, papers: &[PaperAnalysis]) -> SynthesisReport {
    let cleaned = clean_json_response(raw);

    let mut parsed: Option<SynthesisReport> = serde_json::from_str(&cleaned).ok();

    if let Some(ref mut synth) = parsed {
        synth.query = query.to_string();
        synth.ranked_papers = papers.to_vec();
        if !synth.summary.is_empty() && !synth.cross_cutting_themes.is_empty() {
            return synth.clone();
        }
    }

    debug!("Synthesis JSON parse failed or empty, using fallback");
    build_fallback_synthesis(query, papers)
}

fn build_fallback_synthesis(query: &str, papers: &[PaperAnalysis]) -> SynthesisReport {
    let valid: Vec<_> = papers.iter().filter(|p| !p.title.is_empty()).collect();

    if valid.is_empty() {
        let summary = "No papers could be successfully analyzed. The content extraction may have failed due to paywalls, JavaScript requirements, or access restrictions. Try broadening the query or reducing the number of papers.".to_string();
        return SynthesisReport {
            query: query.to_string(),
            cross_cutting_themes: vec![],
            consensus_findings: vec![],
            conflicting_findings: vec![],
            gaps_identified: vec![],
            ranked_papers: papers.to_vec(),
            summary,
        };
    }

    let themes: Vec<String> = extract_themes_from_findings(&valid);

    let findings: Vec<String> = valid
        .iter()
        .flat_map(|p| p.key_findings.iter().cloned())
        .take(10)
        .collect();

    let gaps: Vec<String> = valid
        .iter()
        .filter(|p| {
            let t = p.title.to_lowercase();
            t.contains("survey")
                || t.contains("review")
                || t.contains("comprehensive")
                || t.contains("mapping")
                || t.contains("systematic")
        })
        .map(|p| format!("Gap identified in survey: {}", p.title))
        .take(3)
        .collect();

    let summary = format!(
        "Found {} relevant papers on \"{}\". Key research themes include: {}. Notable findings across papers: {}.",
        valid.len(),
        query,
        themes.join("; "),
        if findings.is_empty() { "further analysis recommended".to_string() }
        else { findings.iter().take(4).cloned().collect::<Vec<_>>().join(". ") }
    );

    let mut ranked = papers.to_vec();
    ranked.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    SynthesisReport {
        query: query.to_string(),
        cross_cutting_themes: themes,
        consensus_findings: findings,
        conflicting_findings: vec![],
        gaps_identified: gaps,
        ranked_papers: ranked,
        summary,
    }
}

fn extract_themes_from_findings(papers: &[&PaperAnalysis]) -> Vec<String> {
    use std::collections::HashMap;

    let mut phrase_counts: HashMap<String, usize> = HashMap::new();
    let stopwords = [
        "the", "a", "an", "in", "on", "of", "for", "to", "and", "is", "are", "this", "that", "it",
        "as", "at", "by", "with", "from", "can", "has", "been", "was", "were", "be", "or", "not",
        "we", "our", "their", "its",
    ];

    for p in papers {
        for finding in &p.key_findings {
            let words: Vec<&str> = finding.split_whitespace().collect();
            for window in words.windows(3) {
                let phrase = window.join(" ");
                let lower = phrase.to_lowercase();
                let first = window[0].to_lowercase();
                let last = window[2].to_lowercase();
                if !stopwords.contains(&first.as_str())
                    && !stopwords.contains(&last.as_str())
                    && phrase.len() > 10
                    && phrase.len() < 60
                {
                    *phrase_counts.entry(lower).or_insert(0) += 1;
                }
            }
        }
    }

    let mut sorted: Vec<_> = phrase_counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    if sorted.len() >= 5 {
        sorted.truncate(5);
    }

    if sorted.is_empty() {
        return papers
            .iter()
            .filter_map(|p| {
                let words: Vec<&str> = p.title.split_whitespace().collect();
                if words.len() >= 4 {
                    Some(words[..words.len().min(4)].join(" "))
                } else {
                    None
                }
            })
            .take(3)
            .collect();
    }

    sorted.into_iter().map(|(phrase, _)| phrase).collect()
}

fn clean_json_response(raw: &str) -> String {
    let mut s = raw.trim().to_string();

    s = s.trim_start_matches("```json").to_string();
    s = s.trim_start_matches("```").to_string();
    s = s.trim_end_matches("```").to_string();
    s = s.trim().to_string();

    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            s = s[start..=end].to_string();
        }
    }

    s
}

fn extract_paper_json_regex(cleaned: &str) -> Option<PaperAnalysis> {
    let title = extract_field(cleaned, "title")?;
    Some(PaperAnalysis {
        source_url: String::new(),
        title,
        authors: extract_field(cleaned, "authors").unwrap_or_default(),
        year: extract_field(cleaned, "year").and_then(|s| s.parse().ok()),
        abstract_text: extract_field(cleaned, "abstract_text").unwrap_or_default(),
        key_findings: extract_array_field(cleaned, "key_findings"),
        methodology: extract_field(cleaned, "methodology").unwrap_or_default(),
        datasets: extract_array_field(cleaned, "datasets"),
        relevance_score: extract_field(cleaned, "relevance_score")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.5),
        relevance_rationale: extract_field(cleaned, "relevance_rationale").unwrap_or_default(),
    })
}

fn extract_field(text: &str, field: &str) -> Option<String> {
    let escaped = regex::escape(field);

    let num_pattern = format!(r#""{}"\s*:\s*([0-9]+\.?[0-9]*)"#, escaped);
    if let Ok(re) = Regex::new(&num_pattern) {
        if let Some(cap) = re.captures(text) {
            if let Some(m) = cap.get(1) {
                return Some(m.as_str().to_string());
            }
        }
    }

    let str_pattern = format!(r#""{}"\s*:\s*"([^"]*)""#, escaped);
    let re = Regex::new(&str_pattern).ok()?;
    re.captures(text)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
}

fn extract_array_field(text: &str, field: &str) -> Vec<String> {
    let pattern = format!(r#""{}"\s*:\s*\[([^\]]*)\]"#, regex::escape(field));
    if let Ok(re) = Regex::new(&pattern) {
        if let Some(cap) = re.captures(text) {
            let inner = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            return inner
                .split(',')
                .map(|s| s.trim().trim_matches('"').trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    vec![]
}
