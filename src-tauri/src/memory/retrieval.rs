// ── Hybrid retrieval ───────────────────────────────────────
// Runs on every user message: embed → Qdrant + graph neighbors → context.

use crate::memory::graph::KnowledgeGraph;
use crate::memory::semantic::SemanticStore;
use crate::memory::episodic::EpisodeStore;
use crate::memory::embeddings::Embedder;
use crate::memory::config::MemoryConfig;
use crate::memory::document_store::DocumentStore;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Serialize, Default)]
pub struct RetrievedContext {
    pub related_facts: Vec<String>,
    pub related_entities: Vec<String>,
    pub related_episodes: Vec<String>,
    pub related_documents: Vec<String>,
}

pub struct HybridRetrieval {
    graph: Arc<Mutex<KnowledgeGraph>>,
    semantic: SemanticStore,
    episode: EpisodeStore,
    embedder: Embedder,
    config: MemoryConfig,
    document_store: Option<Arc<DocumentStore>>,
}

impl HybridRetrieval {
    pub fn new(
        graph: Arc<Mutex<KnowledgeGraph>>,
        semantic: SemanticStore,
        episode: EpisodeStore,
        embedder: Embedder,
        config: MemoryConfig,
    ) -> Self {
        Self { graph, semantic, episode, embedder, config, document_store: None }
    }

    pub fn with_document_store(mut self, ds: Arc<DocumentStore>) -> Self {
        self.document_store = Some(ds);
        self
    }

    pub async fn retrieve(&self, query: &str) -> Result<RetrievedContext, String> {
        let episode_texts = match self.embedder.embed(query).await {
            Ok(_vector) => {
                match self.episode.search_similar(query, 5).await {
                    Ok(eps) => eps.into_iter().map(|e| e.summary).collect(),
                    Err(_) => vec![],
                }
            }
            Err(_) => vec![],
        };

        let query_lower = query.to_lowercase();
        let graph_entities = if query_lower.len() > 3 {
            let graph = self.graph.lock().await;
            graph.neighbors(&query_lower, 2)
        } else {
            vec![]
        };

        let facts = match self.semantic.search_facts(query).await {
            Ok(f) => f.into_iter().map(|f| format!("{} {} {}", f.subject, f.predicate, f.object)).collect(),
            Err(_) => vec![],
        };

        let document_chunks: Vec<String> = match &self.document_store {
            Some(ds) => match ds.search(query, 3).await {
                Ok(results) => results.into_iter().map(|(text, score)| {
                    if score > 0.4 { text } else { String::new() }
                }).filter(|s| !s.is_empty()).collect(),
                Err(_) => vec![],
            },
            None => vec![],
        };

        Ok(RetrievedContext {
            related_facts: facts,
            related_entities: graph_entities,
            related_episodes: episode_texts,
            related_documents: document_chunks,
        })
    }
}
