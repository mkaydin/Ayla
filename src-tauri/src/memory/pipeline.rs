// ── Memory pipeline ────────────────────────────────────────
// Runs after each complete assistant response.
// Summarize → extract → store → embed.

use crate::memory::conversation::ConversationStore;
use crate::memory::episodic::EpisodeStore;
use crate::memory::semantic::SemanticStore;
use crate::memory::graph::{KnowledgeGraph, NodeType};
use crate::memory::summarizer::Summarizer;
use crate::memory::extraction::Extractor;
use crate::memory::working::WorkingMemory;

pub struct MemoryPipeline {
    pub working: WorkingMemory,
    conversation: ConversationStore,
    episode: EpisodeStore,
    semantic: SemanticStore,
    graph: KnowledgeGraph,
    summarizer: Summarizer,
    extractor: Extractor,
}

impl MemoryPipeline {
    pub fn new(
        working: WorkingMemory,
        conversation: ConversationStore,
        episode: EpisodeStore,
        semantic: SemanticStore,
        graph: KnowledgeGraph,
        summarizer: Summarizer,
        extractor: Extractor,
    ) -> Self {
        Self { working, conversation, episode, semantic, graph, summarizer, extractor }
    }

    pub async fn process_turn(
        &mut self,
        user_msg: &str,
        assistant_msg: &str,
    ) -> Result<(), String> {
        // Step 1: Save raw conversation
        self.conversation.save_turn(&self.working.session_id, "user", user_msg).await?;
        self.conversation.save_turn(&self.working.session_id, "assistant", assistant_msg).await?;
        self.working.add_message("user", user_msg);
        self.working.add_message("assistant", assistant_msg);

        let combined = format!("{}\n{}", user_msg, assistant_msg);

        // Step 2: Summarize
        let summary = self.summarizer.summarize_turn(user_msg, assistant_msg).await.unwrap_or_default();

        // Step 3: Extract entities → graph
        if let Ok(entities) = self.extractor.extract_entities(&combined).await {
            for entity in &entities {
                self.graph.upsert_node(entity, NodeType::Concept);
            }
        }

        // Step 4: Extract facts → semantic
        if let Ok(facts) = self.extractor.extract_facts(&combined).await {
            for (subj, pred, obj, conf) in facts {
                let _ = self.semantic.upsert_fact(&subj, &pred, &obj, conf).await;
            }
        }

        // Step 5: Extract relations → graph edges
        if let Ok(relations) = self.extractor.extract_relations(&combined).await {
            for (from, rel, to) in relations {
                let fi = self.graph.upsert_node(&from, NodeType::Concept);
                let ti = self.graph.upsert_node(&to, NodeType::Concept);
                self.graph.upsert_edge(fi, ti, &rel, 0.8);
            }
        }

        // Step 6: Score importance → episodic memory if significant
        if let Ok(importance) = self.summarizer.score_importance(user_msg, assistant_msg).await {
            if importance > 0.4 && !summary.is_empty() {
                let entities: Vec<String> = self.graph.all_nodes().iter()
                    .take(5)
                    .map(|n| n.name.clone())
                    .collect();
                let _ = self.episode.store(&summary, importance, &entities).await;
            }
        }

        // Step 7: Sync graph to SurrealDB
        let _ = self.graph.sync().await;

        Ok(())
    }

    pub async fn end_session(&mut self) -> Result<(), String> {
        let messages: Vec<String> = self.working.get_context_window()
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect();
        let _ = self.summarizer.summarize_session(&messages).await;
        self.working.clear();
        self.graph.sync().await
    }
}
