use crate::models::{SearchResult, SearxngResponse};
use anyhow::{Context, Result};
use url::Url;

const SEARXNG_BASE: &str = "http://localhost:8888";

#[derive(Debug, Clone)]
pub struct Searcher {
    client: reqwest::Client,
    base_url: String,
}

impl Searcher {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (compatible; MetaSearchBot/0.2; +http://localhost)")
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            base_url: SEARXNG_BASE.to_string(),
        })
    }

    pub fn with_base_url(base_url: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (compatible; MetaSearchBot/0.2; +http://localhost)")
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client, base_url })
    }

    pub async fn search(&self, query: &str, pages: u32) -> Result<Vec<SearchResult>> {
        let mut all_results = Vec::new();

        for page in 1..=pages {
            let mut url = Url::parse(&format!("{}/search", self.base_url.trim_end_matches('/')))
                .context("Invalid SearXNG URL")?;
            url.query_pairs_mut()
                .append_pair("q", query)
                .append_pair("format", "json")
                .append_pair("pageno", &page.to_string())
                .append_pair("categories", "science");

            let response = self
                .client
                .get(url)
                .send()
                .await
                .context("Failed to reach SearXNG")?
                .error_for_status()
                .context("SearXNG returned an error status")?;

            let body: SearxngResponse = response
                .json()
                .await
                .context("Failed to parse SearXNG response")?;

            let results: Vec<SearchResult> = body
                .results
                .into_iter()
                .map(|r| {
                    let (is_pdf, rewritten_url) = detect_and_rewrite_pdf(&r.url, &r.content);
                    let final_url = rewritten_url.unwrap_or(r.url);

                    SearchResult {
                        title: r.title,
                        url: final_url,
                        snippet: r.content,
                        engine: r.engine,
                        is_pdf,
                    }
                })
                .collect();

            all_results.extend(results);

            if pages > 1 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }

        Ok(all_results)
    }

    pub async fn search_pdf_only(&self, query: &str, pages: u32) -> Result<Vec<SearchResult>> {
        let pdf_query = format!("{} filetype:pdf", query);
        self.search(&pdf_query, pages).await
    }
}

fn detect_and_rewrite_pdf(url: &str, snippet: &str) -> (bool, Option<String>) {
    let lower_url = url.to_lowercase();
    let lower_snippet = snippet.to_lowercase();

    if lower_url.ends_with(".pdf") || lower_url.ends_with(".pdf?download=true") {
        return (true, None);
    }

    if lower_snippet.contains(".pdf") || lower_snippet.contains("filetype:pdf") {
        return (true, None);
    }

    if let Some(pdf_url) = rewrite_arxiv_to_pdf(url) {
        return (true, Some(pdf_url));
    }

    if let Some(pdf_url) = rewrite_doi_to_pdf(url) {
        return (true, Some(pdf_url));
    }

    let path_pdf_indicators = [
        "/pdf/",
        "/download/pdf/",
        "viewpdf",
        "format=pdf",
        "/fulltext/pdf",
        "download?format=pdf",
        "download/pdf/",
    ];
    for ind in &path_pdf_indicators {
        if lower_url.contains(ind) {
            return (true, None);
        }
    }

    if lower_url.contains("semanticscholar.org") && lower_snippet.contains("view pdf") {
        return (true, None);
    }

    (false, None)
}

fn rewrite_arxiv_to_pdf(url: &str) -> Option<String> {
    if url.contains("arxiv.org/abs/") {
        let id = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("arxiv.org/abs/")
            .trim_end_matches('/')
            .split('v')
            .next()?;

        if !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
        {
            return Some(format!("https://arxiv.org/pdf/{}.pdf", id));
        }
    }

    if url.contains("arxiv.org/pdf/") && !url.ends_with(".pdf") {
        return Some(format!("{}.pdf", url.trim_end_matches('/')));
    }

    None
}

fn rewrite_doi_to_pdf(url: &str) -> Option<String> {
    if url.contains("doi.org/") || url.contains("dx.doi.org/") {
        let doi = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("doi.org/")
            .trim_start_matches("dx.doi.org/");

        if !doi.is_empty() {
            return Some(format!("https://doi.org/{}", doi));
        }
    }
    None
}
