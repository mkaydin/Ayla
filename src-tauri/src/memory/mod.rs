// Temporary Tauri compatibility layer. All backend business logic is now
// compiled from ayla-core; only command transport wrappers remain here.
pub mod commands;

#[allow(unused_imports)]
pub use ayla_core::memory::{
    config, consolidator, context, conversation, document_store, embeddings, episodic, extraction,
    graph, graph_cleanup, ingestion, pipeline, retrieval, semantic, summarizer, working,
};
#[allow(unused_imports)]
pub use ayla_core::memory::{safe_truncate, save_text_store, MemoryManager};
