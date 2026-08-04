// ── Memory consolidator ────────────────────────────────────
// Periodic maintenance: merge duplicates, prune obsolete, reinforce frequent.

use crate::db::surreal::SurrealDB;
use crate::memory::graph::KnowledgeGraph;
use crate::memory::semantic::{Fact, SemanticStore};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Consolidator {
    semantic: SemanticStore,
    graph: Arc<Mutex<KnowledgeGraph>>,
    db: Arc<SurrealDB>,
}

impl Consolidator {
    pub fn new(
        db: Arc<SurrealDB>,
        semantic: SemanticStore,
        graph: Arc<Mutex<KnowledgeGraph>>,
    ) -> Self {
        Self {
            db,
            semantic,
            graph,
        }
    }

    /// Run all consolidation steps. Called periodically.
    pub async fn run(&mut self) -> Result<ConsolidationReport, String> {
        let report = ConsolidationReport {
            merges: self.merge_duplicate_facts().await.unwrap_or(0),
            pruned: self.prune_low_confidence(0.15, 30).await.unwrap_or(0),
            reinforced: self.reinforce_frequent_facts().await.unwrap_or(0),
        };

        // Sync graph to DB after changes
        let sync_result = {
            let mut graph = self.graph.lock().await;
            graph.sync().await
        };
        if let Err(e) = sync_result {
            log::warn!("Consolidation graph sync failed: {}", e);
        }

        Ok(report)
    }

    /// Merge facts with the same subject+predicate+object but different IDs.
    async fn merge_duplicate_facts(&mut self) -> Result<usize, String> {
        let facts = self.semantic.get_all_facts().await?;
        if facts.is_empty() {
            return Ok(0);
        }

        // Group by (subject, predicate, object)
        let mut groups: HashMap<(String, String, String), Vec<Fact>> = HashMap::new();
        for fact in facts {
            let key = (
                fact.subject.to_lowercase(),
                fact.predicate.to_lowercase(),
                fact.object.to_lowercase(),
            );
            groups.entry(key).or_default().push(fact);
        }

        let mut merged = 0;

        for (_, group) in groups {
            if group.len() < 2 {
                continue;
            }

            // Keep the one with highest count, merge others into it
            let mut sorted = group.clone();
            sorted.sort_by_key(|fact| std::cmp::Reverse(fact.count));

            let keeper = &sorted[0];
            let keeper_id = keeper.id.clone().unwrap_or_default();

            let total_count: u32 = sorted.iter().map(|f| f.count).sum();
            let avg_confidence: f32 =
                sorted.iter().map(|f| f.confidence).sum::<f32>() / sorted.len() as f32;
            let newest = sorted
                .iter()
                .map(|f| f.last_seen.clone())
                .max()
                .unwrap_or(keeper.last_seen.clone());

            // Update the keeper
            let update_sql = format!(
                "UPDATE {} SET confidence = {}, count = {}, last_seen = '{}';",
                keeper_id,
                (avg_confidence * 1.1).min(1.0),
                total_count,
                newest,
            );
            let _ = self.db.query(&update_sql).await;

            // Delete the rest
            for fact in &sorted[1..] {
                if let Some(ref id) = fact.id {
                    let _ = self.semantic.delete_fact(id).await;
                    merged += 1;
                }
            }
        }

        Ok(merged)
    }

    /// Remove facts with confidence below threshold that haven't been updated recently.
    async fn prune_low_confidence(
        &self,
        threshold: f32,
        max_age_days: i64,
    ) -> Result<usize, String> {
        let facts = self.semantic.get_all_facts().await?;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days);
        let cutoff_str = cutoff.to_rfc3339();

        let mut pruned = 0;

        for fact in facts {
            if fact.confidence >= threshold {
                continue;
            }

            let age_ok = fact.last_seen <= cutoff_str;
            if !age_ok {
                continue;
            }

            if let Some(ref id) = fact.id {
                let _ = self.semantic.delete_fact(id).await;
                pruned += 1;
            }
        }

        Ok(pruned)
    }

    /// Increase confidence for facts that have been observed many times.
    async fn reinforce_frequent_facts(&self) -> Result<usize, String> {
        let facts = self.semantic.get_all_facts().await?;
        let mut reinforced = 0;

        for fact in facts {
            if fact.count < 3 {
                continue;
            }
            if fact.confidence >= 0.95 {
                continue;
            }

            // Boost confidence by 0.05 per observation above 2, capped at 0.95
            let boost = ((fact.count as f32 - 2.0) * 0.05).min(0.25);
            let new_conf = (fact.confidence + boost).min(0.95);

            if let Some(ref id) = fact.id {
                let update_sql = format!("UPDATE {} SET confidence = {};", id, new_conf);
                let _ = self.db.query(&update_sql).await;
                reinforced += 1;
            }
        }

        Ok(reinforced)
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ConsolidationReport {
    pub merges: usize,
    pub pruned: usize,
    pub reinforced: usize,
}
