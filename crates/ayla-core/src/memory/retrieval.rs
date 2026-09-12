// ── Hybrid retrieval ───────────────────────────────────────
// Runs on every user message: embed → Qdrant + graph neighbors → context.

use super::{code_analysis_store::CodeAnalysisVectorStore, graph_v2::GraphV2Store};
use crate::memory::config::MemoryConfig;
use crate::memory::document_store::DocumentStore;
use crate::memory::episodic::EpisodeStore;
use crate::memory::graph::KnowledgeGraph;
use crate::memory::semantic::SemanticStore;
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
    config: MemoryConfig,
    document_store: Option<Arc<DocumentStore>>,
    code_store: Option<Arc<CodeAnalysisVectorStore>>,
    graph_v2: Option<Arc<Mutex<GraphV2Store>>>,
}

impl HybridRetrieval {
    pub fn new(
        graph: Arc<Mutex<KnowledgeGraph>>,
        semantic: SemanticStore,
        episode: EpisodeStore,
        config: MemoryConfig,
    ) -> Self {
        Self {
            graph,
            semantic,
            episode,
            config,
            document_store: None,
            code_store: None,
            graph_v2: None,
        }
    }

    pub fn with_document_store(mut self, ds: Arc<DocumentStore>) -> Self {
        self.document_store = Some(ds);
        self
    }

    pub fn with_structured_knowledge(
        mut self,
        graph: Arc<Mutex<GraphV2Store>>,
        code: Arc<CodeAnalysisVectorStore>,
    ) -> Self {
        self.graph_v2 = Some(graph);
        self.code_store = Some(code);
        self
    }

    pub async fn retrieve(&self, query: &str) -> Result<RetrievedContext, String> {
        let graph_entities = if let Some(store) = &self.graph_v2 {
            store.lock().await.context_for_query(query, 20).await?
        } else if query.len() > 3 {
            self.graph.lock().await.entity_context_for_query(query, 2)
        } else {
            vec![]
        };
        let episode_future = self
            .episode
            .search_similar(query, self.config.top_k_retrieval as u64);
        let facts_future = self.semantic.search_facts(query);
        let document_future = async {
            match &self.document_store {
                Some(ds) => ds.search(query, 3).await.unwrap_or_default(),
                None => vec![],
            }
        };
        let code_future = async {
            match &self.code_store {
                Some(store) => store.search(query, 3).await.unwrap_or_default(),
                None => vec![],
            }
        };
        let (episodes, facts_result, documents, code) =
            tokio::join!(episode_future, facts_future, document_future, code_future);

        let episode_texts = episodes
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.summary)
            .collect();
        let facts = match facts_result {
            Ok(f) => f
                .into_iter()
                .map(|f| format!("{} {} {}", f.subject, f.predicate, f.object))
                .collect(),
            Err(_) => vec![],
        };
        let document_chunks = documents
            .into_iter()
            .chain(code)
            .filter_map(|(text, score)| (score > 0.4).then_some(text))
            .collect();

        Ok(RetrievedContext {
            related_facts: facts,
            related_entities: graph_entities,
            related_episodes: episode_texts,
            related_documents: document_chunks,
        })
    }
}
