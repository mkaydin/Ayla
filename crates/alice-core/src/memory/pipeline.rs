// ── Memory pipeline ────────────────────────────────────────
// Runs after each complete assistant response.
// Summarize → extract → store → embed.

use crate::memory::conversation::ConversationStore;
use crate::memory::episodic::EpisodeStore;
use crate::memory::extraction::Extractor;
use crate::memory::graph::{KnowledgeGraph, NodeType};
use crate::memory::semantic::SemanticStore;
use crate::memory::summarizer::Summarizer;
use crate::memory::working::WorkingMemory;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct MemoryPipeline {
    pub working: WorkingMemory,
    conversation: ConversationStore,
    episode: EpisodeStore,
    semantic: SemanticStore,
    graph: Arc<Mutex<KnowledgeGraph>>,
    summarizer: Summarizer,
    extractor: Extractor,
}

impl MemoryPipeline {
    pub fn new(
        working: WorkingMemory,
        conversation: ConversationStore,
        episode: EpisodeStore,
        semantic: SemanticStore,
        graph: Arc<Mutex<KnowledgeGraph>>,
        summarizer: Summarizer,
        extractor: Extractor,
    ) -> Self {
        Self {
            working,
            conversation,
            episode,
            semantic,
            graph,
            summarizer,
            extractor,
        }
    }

    pub async fn process_turn(
        &mut self,
        user_msg: &str,
        assistant_msg: &str,
    ) -> Result<(), String> {
        // Step 1: Save raw conversation
        self.conversation
            .save_turn(&self.working.session_id, "user", user_msg)
            .await?;
        self.conversation
            .save_turn(&self.working.session_id, "assistant", assistant_msg)
            .await?;
        self.working.add_message("user", user_msg);
        self.working.add_message("assistant", assistant_msg);

        let combined = format!("{}\n{}", user_msg, assistant_msg);

        // Run independent LLM work concurrently and use one consolidated
        // extraction request. Previously a turn made five sequential model calls
        // and held the graph mutex across most of them.
        let (summary_result, extract_result, importance_result) = tokio::join!(
            self.summarizer.summarize_turn(user_msg, assistant_msg),
            self.extractor.extract_all(&combined),
            self.summarizer.score_importance(user_msg, assistant_msg),
        );
        let summary = summary_result.unwrap_or_default();
        let importance = importance_result.unwrap_or(0.0);
        let extract = extract_result.unwrap_or_else(|e| {
            log::warn!("Turn extraction failed: {}", e);
            crate::memory::extraction::KnowledgeExtract {
                entities: vec![],
                facts: vec![],
                relations: vec![],
            }
        });

        for (subj, pred, obj, conf) in &extract.facts {
            if let Err(e) = self.semantic.upsert_fact(subj, pred, obj, *conf).await {
                log::warn!("Fact store failed: {}", e);
            }
        }

        {
            let mut graph = self.graph.lock().await;
            for entity in &extract.entities {
                graph.upsert_node(&entity.name, entity.node_type.clone());
            }
            for relation in &extract.relations {
                let from_type = extract
                    .entities
                    .iter()
                    .find(|entity| entity.name.eq_ignore_ascii_case(&relation.from))
                    .map(|entity| entity.node_type.clone())
                    .unwrap_or(NodeType::Concept);
                let to_type = extract
                    .entities
                    .iter()
                    .find(|entity| entity.name.eq_ignore_ascii_case(&relation.to))
                    .map(|entity| entity.node_type.clone())
                    .unwrap_or(NodeType::Concept);
                let fi = graph.upsert_node(&relation.from, from_type);
                let ti = graph.upsert_node(&relation.to, to_type);
                graph.upsert_edge(fi, ti, &relation.relation, relation.confidence);
            }
            graph.sync().await?;
        }

        if importance > 0.4 && !summary.is_empty() {
            let entity_names: Vec<String> = extract
                .entities
                .iter()
                .map(|entity| entity.name.clone())
                .collect();
            if let Err(e) = self
                .episode
                .store(&summary, importance, &entity_names)
                .await
            {
                log::warn!("Episode store failed: {}", e);
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn end_session(&mut self) -> Result<(), String> {
        let messages: Vec<String> = self
            .working
            .get_context_window()
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect();
        let _ = self.summarizer.summarize_session(&messages).await;
        self.working.clear();
        let mut graph = self.graph.lock().await;
        graph.sync().await
    }
}
