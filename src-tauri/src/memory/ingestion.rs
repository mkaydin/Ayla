// ── Ingestion pipeline ─────────────────────────────────────
// Bridges tool results into memory stores.

use crate::memory::conversation::ConversationStore;
use crate::memory::episodic::EpisodeStore;
use crate::memory::semantic::SemanticStore;
use crate::memory::graph::{KnowledgeGraph, NodeType};
use crate::memory::extraction::Extractor;
use crate::memory::summarizer::Summarizer;
use crate::tools::types::IngestionResult;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct IngestionPipeline {
    conversation: ConversationStore,
    episode: EpisodeStore,
    semantic: SemanticStore,
    graph: Arc<Mutex<KnowledgeGraph>>,
    extractor: Extractor,
    summarizer: Summarizer,
}

impl IngestionPipeline {
    pub fn new(
        conversation: ConversationStore,
        episode: EpisodeStore,
        semantic: SemanticStore,
        graph: Arc<Mutex<KnowledgeGraph>>,
        extractor: Extractor,
        summarizer: Summarizer,
    ) -> Self {
        Self { conversation, episode, semantic, graph, extractor, summarizer }
    }

    pub async fn ingest(&mut self, result: IngestionResult) -> Result<String, String> {
        let session_id = format!("ingest_{}", chrono::Utc::now().timestamp());

        let _ = self.conversation.save_turn(&session_id, "tool", &result.full_text).await;

        let mut entity_count = 0;
        if let Ok(entities) = self.extractor.extract_entities(&result.full_text).await {
            entity_count = entities.len();
            log::info!("Ingestion: extracted {} entities", entity_count);
            let mut g = self.graph.lock().await;
            for entity in &entities {
                g.upsert_node(entity, NodeType::Concept);
            }
        }

        let mut fact_count = 0;
        if let Ok(facts) = self.extractor.extract_facts(&result.full_text).await {
            fact_count = facts.len();
            log::info!("Ingestion: extracted {} facts", fact_count);
            for (subj, pred, obj, conf) in facts {
                let _ = self.semantic.upsert_fact(&subj, &pred, &obj, conf).await;
            }
        }

        let mut edge_count = 0;
        if let Ok(relations) = self.extractor.extract_relations(&result.full_text).await {
            edge_count = relations.len();
            log::info!("Ingestion: extracted {} relations", edge_count);
            let mut g = self.graph.lock().await;
            for (from, rel, to) in relations {
                let fi = g.upsert_node(&from, NodeType::Concept);
                let ti = g.upsert_node(&to, NodeType::Concept);
                g.upsert_edge(fi, ti, &rel, 0.8);
            }
        }

        let summary = self.summarizer.summarize_turn(
            &result.source.input,
            &result.full_text[..result.full_text.len().min(4000)],
        ).await.unwrap_or_default();

        if !summary.is_empty() {
            let _ = self.episode.store(&summary, 0.7, &result.entities).await;
        }

        let mut g = self.graph.lock().await;
        let _ = g.sync().await;
        let node_count = g.node_count();
        let edge_count = g.edge_count();
        Ok(format!("Ingested: {} nodes, {} edges in graph. Entities: {}, facts: {}, relations: {}.",
            node_count, edge_count, entity_count, fact_count, edge_count))
    }
}
