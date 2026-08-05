use crate::ai::AiClient;
use crate::downloader::Downloader;
use crate::extractor;
use crate::models::{
    ExtractedSource, PaperAnalysis, RenderedPage, SearchResult, SessionReport, SynthesisReport,
};
use crate::renderer::Renderer;
use crate::searcher::Searcher;
use crate::synthesizer::Synthesizer;
use crate::{ProgressCallback, ResearchProgress};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};
use uuid::Uuid;

const MAX_CONCURRENT_DOWNLOADS: usize = 10;
const MAX_CONCURRENT_RENDERS: usize = 6;
const MAX_TEXT_EXTRACT_CHARS: usize = 80000;

#[derive(Debug, Clone)]
pub struct Orchestrator {
    searcher: Searcher,
    downloader: Downloader,
    renderer: Renderer,
    ai: AiClient,
    download_dir: PathBuf,
    report_dir: PathBuf,
}

impl Orchestrator {
    fn notify(
        progress: Option<&ProgressCallback<'_>>,
        stage: &str,
        percent: u8,
        message: impl Into<String>,
    ) {
        if let Some(progress) = progress {
            progress(ResearchProgress {
                stage: stage.into(),
                percent,
                message: message.into(),
            });
        }
    }

    pub fn new(
        searcher: Searcher,
        downloader: Downloader,
        renderer: Renderer,
        ai: AiClient,
    ) -> Result<Self> {
        Self::with_data_dir(
            searcher,
            downloader,
            renderer,
            ai,
            std::env::temp_dir().join("ayla-research"),
        )
    }

    pub fn with_data_dir(
        searcher: Searcher,
        downloader: Downloader,
        renderer: Renderer,
        ai: AiClient,
        data_dir: PathBuf,
    ) -> Result<Self> {
        let download_dir = data_dir.join("downloads");
        let report_dir = data_dir.join("reports");

        std::fs::create_dir_all(&download_dir)?;
        std::fs::create_dir_all(&report_dir)?;

        Ok(Self {
            searcher,
            downloader,
            renderer,
            ai,
            download_dir,
            report_dir,
        })
    }

    pub async fn run(
        &self,
        query: &str,
        max_papers: usize,
        search_pages: u32,
    ) -> Result<SessionReport> {
        self.run_with_progress(query, max_papers, search_pages, None)
            .await
    }

    pub async fn run_with_progress(
        &self,
        query: &str,
        max_papers: usize,
        search_pages: u32,
        progress: Option<&ProgressCallback<'_>>,
    ) -> Result<SessionReport> {
        let session_id = Uuid::new_v4();
        let session_dir = self.download_dir.join(session_id.to_string());
        tokio::fs::create_dir_all(&session_dir).await?;

        info!("Session {}: Searching for: {}", session_id, query);
        Self::notify(progress, "enriching", 5, "Enriching the research query");

        let enriched = self.ai.enrich_query(query).await.unwrap_or_else(|e| {
            warn!("Query enrichment failed: {}. Using raw query.", e);
            query.to_string()
        });
        info!("Enriched query: {}", enriched);

        Self::notify(progress, "searching", 15, "Querying scientific sources");
        let results = self
            .searcher
            .search(&enriched, search_pages)
            .await
            .context("Search failed")?;

        info!("Found {} total results", results.len());

        let pdf_urls: Vec<String> = results
            .iter()
            .filter(|r| r.is_pdf)
            .map(|r| r.url.clone())
            .collect();

        let web_urls: Vec<&SearchResult> = results.iter().filter(|r| !r.is_pdf).collect();

        info!(
            "Detected {} PDF links, {} web links",
            pdf_urls.len(),
            web_urls.len()
        );

        Self::notify(progress, "downloading", 30, "Downloading candidate papers");
        let downloaded = self.download_pdfs(&pdf_urls, &session_dir).await;

        Self::notify(progress, "rendering", 45, "Rendering web sources");
        let rendered = self.render_web_pages(&web_urls, max_papers).await;

        let mut all_sources: Vec<ExtractedSource> = Vec::new();

        for pdf in &downloaded {
            match extractor::extract_pdf_text(pdf, MAX_TEXT_EXTRACT_CHARS) {
                Ok(source) => all_sources.push(source),
                Err(e) => {
                    warn!("PDF extraction failed for {}: {}", pdf.original_url, e);
                }
            }
        }

        for page in &rendered {
            all_sources.push(extractor::extract_from_rendered(
                &page.url,
                &page.text_content,
            ));
        }

        info!("Extracted text from {} sources", all_sources.len());

        Self::notify(progress, "analyzing", 60, "Analyzing extracted sources");
        let analyses = self
            .analyze_sources(&all_sources, &enriched, max_papers)
            .await;

        let ranked = self
            .ai
            .score_relevance_batch(&enriched, &analyses)
            .await
            .unwrap_or(analyses);

        Self::notify(
            progress,
            "synthesizing",
            82,
            "Synthesizing research findings",
        );
        let synthesizer = Synthesizer::new(self.ai.clone());
        let synthesis = synthesizer
            .synthesize(&enriched, &ranked)
            .await
            .unwrap_or_else(|e| {
                error!("Synthesis failed: {}", e);
                SynthesisReport {
                    query: enriched.clone(),
                    cross_cutting_themes: vec![],
                    consensus_findings: vec![],
                    conflicting_findings: vec![],
                    gaps_identified: vec![],
                    ranked_papers: ranked.clone(),
                    summary: format!("Synthesis error: {}", e),
                }
            });

        let report = SessionReport {
            session_id,
            query: query.to_string(),
            enriched_query: enriched,
            total_results_found: results.len(),
            pdfs_downloaded: downloaded.len(),
            pages_rendered: rendered.len(),
            papers_analyzed: ranked,
            synthesis,
            download_dir: session_dir,
        };

        Self::notify(progress, "saving", 95, "Saving the research report");
        self.save_report(&report).await?;
        Self::notify(progress, "complete", 100, "Research complete");

        Ok(report)
    }

    async fn download_pdfs(
        &self,
        urls: &[String],
        session_dir: &Path,
    ) -> Vec<crate::models::DownloadedPdf> {
        if urls.is_empty() {
            return vec![];
        }

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS));
        let handles: Vec<_> = urls
            .iter()
            .map(|url| {
                let downloader = self.downloader.clone();
                let url = url.clone();
                let session_dir = session_dir.to_path_buf();
                let sem = semaphore.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await;
                    downloader.download_pdf(&url, &session_dir).await
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(pdf)) => results.push(pdf),
                Ok(Err(e)) => warn!("PDF download failed: {}", e),
                Err(e) => warn!("Download task panicked: {}", e),
            }
        }
        results
    }

    async fn render_web_pages(
        &self,
        results: &[&SearchResult],
        max_pages: usize,
    ) -> Vec<RenderedPage> {
        if results.is_empty() {
            return vec![];
        }

        let to_render: Vec<_> = results.iter().take(max_pages).collect();

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_RENDERS));
        let handles: Vec<_> = to_render
            .iter()
            .map(|result| {
                let renderer = self.renderer.clone();
                let url = result.url.clone();
                let sem = semaphore.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await;
                    renderer.fetch_rendered(&url).await
                })
            })
            .collect();

        let mut rendered = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(result)) => rendered.push(result.page),
                Ok(Err(e)) => warn!("Render failed: {}", e),
                Err(e) => warn!("Render task panicked: {}", e),
            }
        }
        rendered
    }

    async fn analyze_sources(
        &self,
        sources: &[ExtractedSource],
        query: &str,
        max_papers: usize,
    ) -> Vec<PaperAnalysis> {
        if sources.is_empty() {
            return vec![];
        }

        let to_analyze: Vec<_> = sources
            .iter()
            .filter(|s| s.text.len() > 100)
            .take(max_papers)
            .collect();

        let semaphore = Arc::new(Semaphore::new(self.ai.recommended_concurrency()));
        let handles: Vec<_> = to_analyze
            .iter()
            .map(|source| {
                let ai = self.ai.clone();
                let url = source.source_url.clone();
                let text = source.text.clone();
                let query = query.to_string();
                let sem = semaphore.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await;
                    ai.analyze_paper(&url, &text, &query).await
                })
            })
            .collect();

        let mut analyses = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(analysis)) => analyses.push(analysis),
                Ok(Err(e)) => warn!("Paper analysis failed: {}", e),
                Err(e) => warn!("Analysis task panicked: {}", e),
            }
        }
        analyses
    }

    async fn save_report(&self, report: &SessionReport) -> Result<()> {
        let report_path = self.report_dir.join(format!("{}.json", report.session_id));
        let json = serde_json::to_string_pretty(report)?;
        tokio::fs::write(&report_path, &json).await?;
        info!("Report saved to: {}", report_path.display());

        let md_path = self.report_dir.join(format!("{}.md", report.session_id));
        self.save_markdown_report(report, &md_path).await?;
        info!("Markdown report saved to: {}", md_path.display());

        Ok(())
    }

    async fn save_markdown_report(&self, report: &SessionReport, path: &Path) -> Result<()> {
        let mut md = String::new();

        md.push_str(&format!(
            "# Scientific Search Report\n\n\
             **Query:** {}\n\
             **Enriched Query:** {}\n\
             **Session ID:** {}\n\
             **Found:** {} results | **Downloaded:** {} PDFs | **Rendered:** {} pages | **Analyzed:** {} papers\n\n\
             ---\n\n",
            report.query,
            report.enriched_query,
            report.session_id,
            report.total_results_found,
            report.pdfs_downloaded,
            report.pages_rendered,
            report.papers_analyzed.len(),
        ));

        md.push_str("## Synthesis Summary\n\n");
        md.push_str(&report.synthesis.summary);
        md.push_str("\n\n");

        if !report.synthesis.cross_cutting_themes.is_empty() {
            md.push_str("### Cross-Cutting Themes\n");
            for theme in &report.synthesis.cross_cutting_themes {
                md.push_str(&format!("- {}\n", theme));
            }
            md.push('\n');
        }

        if !report.synthesis.consensus_findings.is_empty() {
            md.push_str("### Areas of Consensus\n");
            for finding in &report.synthesis.consensus_findings {
                md.push_str(&format!("- {}\n", finding));
            }
            md.push('\n');
        }

        if !report.synthesis.conflicting_findings.is_empty() {
            md.push_str("### Conflicting Findings\n");
            for conflict in &report.synthesis.conflicting_findings {
                md.push_str(&format!("- {}\n", conflict));
            }
            md.push('\n');
        }

        if !report.synthesis.gaps_identified.is_empty() {
            md.push_str("### Research Gaps\n");
            for gap in &report.synthesis.gaps_identified {
                md.push_str(&format!("- {}\n", gap));
            }
            md.push('\n');
        }

        md.push_str("---\n\n## Ranked Papers\n\n");
        for (i, paper) in report.synthesis.ranked_papers.iter().enumerate() {
            md.push_str(&format!(
                "### {}. {} (Score: {:.0}%)\n\
                 **Authors:** {}\n\
                 **Year:** {}\n\
                 **URL:** {}\n\n\
                 **Abstract:** {}\n\n\
                 **Key Findings:**\n",
                i + 1,
                paper.title,
                paper.relevance_score * 100.0,
                paper.authors,
                paper.year.map_or("N/A".to_string(), |y| y.to_string()),
                paper.source_url,
                paper.abstract_text.chars().take(800).collect::<String>(),
            ));
            for finding in &paper.key_findings {
                md.push_str(&format!("- {}\n", finding));
            }
            md.push_str(&format!("\n**Methodology:** {}\n\n", paper.methodology));
            if !paper.datasets.is_empty() {
                md.push_str(&format!("**Datasets:** {}\n\n", paper.datasets.join(", ")));
            }
            md.push_str("---\n\n");
        }

        if !report.papers_analyzed.is_empty() {
            md.push_str("## Downloaded PDFs\n\n");
            for paper in &report.papers_analyzed {
                md.push_str(&format!("- **{}**: {}\n", paper.title, paper.source_url));
            }
        }

        tokio::fs::write(path, &md).await?;
        Ok(())
    }
}
