// ── Ingestion pipeline ─────────────────────────────────────
// Bridges tool results into memory stores.

use crate::memory::conversation::ConversationStore;
use crate::memory::episodic::EpisodeStore;
use crate::memory::extraction::Extractor;
use crate::memory::graph::{KnowledgeGraph, NodeType};
use crate::memory::semantic::SemanticStore;
use crate::memory::summarizer::Summarizer;
use crate::tools::types::IngestionResult;
use crate::EventSink;
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
        Self {
            conversation,
            episode,
            semantic,
            graph,
            extractor,
            summarizer,
        }
    }

    pub async fn ingest(
        &mut self,
        result: IngestionResult,
        event_sink: Option<&dyn EventSink>,
    ) -> Result<String, String> {
        let session_id = format!("ingest_{}", chrono::Utc::now().timestamp());

        let _ = self
            .conversation
            .save_turn(&session_id, "tool", &result.full_text)
            .await;
        if let Some(events) = event_sink {
            let _ = events.emit("ingest-progress", serde_json::json!({"step": "extracting", "pct": 30, "source": crate::memory::safe_truncate(&result.full_text, 40)}));
        }

        let source_name = result.source.input.clone();
        let extract = match self
            .extractor
            .extract_document_with_progress(&result.full_text, &source_name, event_sink)
            .await
        {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Ingestion: extraction failed: {}", e);
                return Err(format!("Extraction failed: {}", e));
            }
        };

        if let Some(events) = event_sink {
            let _ = events.emit(
                "ingest-progress",
                serde_json::json!({"step": "building_graph", "pct": 60}),
            );
        }

        let entity_count = extract.entities.len();
        let fact_count = extract.facts.len();
        let relation_count = extract.relations.len();

        // Store facts
        for (subj, pred, obj, conf) in &extract.facts {
            let _ = self.semantic.upsert_fact(subj, pred, obj, *conf).await;
        }

        // Persist typed nodes and semantic relations. Every extracted entity also
        // receives a provenance edge from the source document, ensuring the graph
        // remains navigable even when a relation is only implied across chunks.
        {
            let mut g = self.graph.lock().await;
            let document_index =
                g.upsert_node_sourced(&source_name, NodeType::Document, &source_name);
            for entity in &extract.entities {
                let entity_index =
                    g.upsert_node_sourced(&entity.name, entity.node_type.clone(), &source_name);
                if entity_index != document_index {
                    g.upsert_edge_sourced(
                        document_index,
                        entity_index,
                        "MENTIONS",
                        1.0,
                        &source_name,
                    );
                }
            }
            for relation in &extract.relations {
                let from = g.upsert_node_sourced(
                    &relation.from,
                    entity_type(&extract.entities, &relation.from),
                    &source_name,
                );
                let to = g.upsert_node_sourced(
                    &relation.to,
                    entity_type(&extract.entities, &relation.to),
                    &source_name,
                );
                g.upsert_edge_sourced(
                    from,
                    to,
                    &relation.relation,
                    relation.confidence,
                    &source_name,
                );
            }
        }

        if let Some(events) = event_sink {
            let _ = events.emit(
                "ingest-progress",
                serde_json::json!({"step": "summarizing", "pct": 90}),
            );
        }

        let summary = self
            .summarizer
            .summarize_turn(
                &result.source.input,
                crate::memory::safe_truncate(&result.full_text, 4000),
            )
            .await
            .unwrap_or_default();

        if !summary.is_empty() {
            let entity_names: Vec<String> = extract
                .entities
                .iter()
                .map(|entity| entity.name.clone())
                .collect();
            let _ = self.episode.store(&summary, 0.7, &entity_names).await;
        }

        let mut g = self.graph.lock().await;
        let _ = g.sync().await;
        let node_count = g.node_count();
        let edge_count = g.edge_count();
        Ok(format!(
            "Ingested: {} nodes, {} edges. Entities: {}, facts: {}, relations: {}.",
            node_count, edge_count, entity_count, fact_count, relation_count
        ))
    }
}

fn entity_type(entities: &[crate::memory::extraction::ExtractedEntity], name: &str) -> NodeType {
    entities
        .iter()
        .find(|entity| entity.name.eq_ignore_ascii_case(name))
        .map(|entity| entity.node_type.clone())
        .unwrap_or(NodeType::Concept)
}
