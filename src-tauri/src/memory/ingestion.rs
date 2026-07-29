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
use tauri::Emitter;

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

    pub async fn ingest(&mut self, result: IngestionResult, app_handle: Option<&tauri::AppHandle>) -> Result<String, String> {
        let session_id = format!("ingest_{}", chrono::Utc::now().timestamp());

        let _ = self.conversation.save_turn(&session_id, "tool", &result.full_text).await;
        if let Some(h) = app_handle {
            let _ = h.emit("ingest-progress", serde_json::json!({"step": "extracting", "pct": 30, "source": &result.full_text[..40.min(result.full_text.len())]}));
        }

        // Single LLM call: entities + relations + facts together
        let extract = match self.extractor.extract_all(&result.full_text).await {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Ingestion: extraction failed: {}", e);
                return Err(format!("Extraction failed: {}", e));
            }
        };

        if let Some(h) = app_handle {
            let _ = h.emit("ingest-progress", serde_json::json!({"step": "building_graph", "pct": 60}));
        }

        let entity_count = extract.entities.len();
        let fact_count = extract.facts.len();
        let relation_count = extract.relations.len();

        // Add entities as graph nodes, tagged with source
        let source_name = result.source.input.clone();
        {
            let mut g = self.graph.lock().await;
            for entity in &extract.entities {
                g.upsert_node_sourced(entity, NodeType::Concept, &source_name);
            }
        }

        // Store facts
        for (subj, pred, obj, conf) in &extract.facts {
            let _ = self.semantic.upsert_fact(subj, pred, obj, *conf).await;
        }

        // Add relations as graph edges (node upsert happens here to ensure both sides exist)
        {
            let mut g = self.graph.lock().await;
            for (from, rel, to) in &extract.relations {
                let fi = g.upsert_node_sourced(from, NodeType::Concept, &source_name);
                let ti = g.upsert_node_sourced(to, NodeType::Concept, &source_name);
                g.upsert_edge(fi, ti, rel, 0.8);
            }
            // Remove orphan nodes (no edges) — they add noise
            let removed = g.remove_orphans();
            if removed > 0 {
                log::info!("Ingestion: removed {} orphan nodes (no relations)", removed);
            }
        }

        if let Some(h) = app_handle {
            let _ = h.emit("ingest-progress", serde_json::json!({"step": "summarizing", "pct": 90}));
        }

        let summary = self.summarizer.summarize_turn(
            &result.source.input,
            &result.full_text[..result.full_text.len().min(4000)],
        ).await.unwrap_or_default();

        if !summary.is_empty() {
            let _ = self.episode.store(&summary, 0.7, &extract.entities).await;
        }

        let mut g = self.graph.lock().await;
        let _ = g.sync().await;
        let node_count = g.node_count();
        let edge_count = g.edge_count();
        Ok(format!("Ingested: {} nodes, {} edges. Entities: {}, facts: {}, relations: {}.",
            node_count, edge_count, entity_count, fact_count, relation_count))
    }
}
