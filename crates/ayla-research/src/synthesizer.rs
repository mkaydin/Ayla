use crate::ai::AiClient;
use crate::models::{PaperAnalysis, SynthesisReport};
use anyhow::Result;
use tracing::info;

pub struct Synthesizer {
    ai: AiClient,
}

impl Synthesizer {
    pub fn new(ai: AiClient) -> Self {
        Self { ai }
    }

    pub async fn synthesize(
        &self,
        query: &str,
        papers: &[PaperAnalysis],
    ) -> Result<SynthesisReport> {
        info!("Synthesizing {} papers for query: {}", papers.len(), query);
        let mut report = self.ai.synthesize(query, papers).await?;

        let mut scored = papers.to_vec();
        scored.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        report.ranked_papers = scored;

        Ok(report)
    }
}
