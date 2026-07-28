// ── Hybrid retrieval ───────────────────────────────────────
// Runs on every user message: embed → Qdrant + graph neighbors → context.

use crate::memory::graph::KnowledgeGraph;
use crate::memory::semantic::SemanticStore;
use crate::memory::episodic::EpisodeStore;
use crate::memory::embeddings::Embedder;
use crate::memory::config::MemoryConfig;
use serde::Serialize;

#[derive(Serialize, Default)]
pub struct RetrievedContext {
    pub related_facts: Vec<String>,
    pub related_entities: Vec<String>,
    pub related_episodes: Vec<String>,
}

pub struct HybridRetrieval {
    graph: KnowledgeGraph,
    semantic: SemanticStore,
    episode: EpisodeStore,
    embedder: Embedder,
    config: MemoryConfig,
}

impl HybridRetrieval {
    pub fn new(
        graph: KnowledgeGraph,
        semantic: SemanticStore,
        episode: EpisodeStore,
        embedder: Embedder,
        config: MemoryConfig,
    ) -> Self {
        Self { graph, semantic, episode, embedder, config }
    }

    pub async fn retrieve(&self, query: &str) -> Result<RetrievedContext, String> {
        // 1. Embed query → Qdrant search
        let episode_texts = match self.embedder.embed(query).await {
            Ok(_vector) => {
                match self.episode.search_similar(query, 5).await {
                    Ok(eps) => eps.into_iter().map(|e| e.summary).collect(),
                    Err(_) => vec![],
                }
            }
            Err(_) => vec![],
        };

        // 2. Simple keyword-based entity extraction from query
        let query_lower = query.to_lowercase();
        let graph_entities = if query_lower.len() > 3 {
            self.graph.neighbors(&query_lower, 2)
        } else {
            vec![]
        };

        // 3. Fact search
        let facts = match self.semantic.search_facts(query).await {
            Ok(f) => f.into_iter().map(|f| format!("{} {} {}", f.subject, f.predicate, f.object)).collect(),
            Err(_) => vec![],
        };

        Ok(RetrievedContext {
            related_facts: facts,
            related_entities: graph_entities,
            related_episodes: episode_texts,
        })
    }
}
