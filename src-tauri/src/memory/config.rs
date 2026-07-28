// ── Memory configuration ────────────────────────────────────

#[derive(Clone)]
pub struct MemoryConfig {
    pub surrealdb_url: String,
    pub surrealdb_ns: String,
    pub surrealdb_db: String,
    pub qdrant_url: String,
    pub ollama_url: String,
    pub embed_model: String,
    pub llm_model: String,
    pub embed_dim: usize,
    pub top_k_retrieval: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            surrealdb_url: "http://127.0.0.1:8000".into(),
            surrealdb_ns: "alice".into(),
            surrealdb_db: "memory".into(),
            qdrant_url: "http://127.0.0.1:6334".into(),
            ollama_url: "http://127.0.0.1:11434".into(),
            embed_model: "qwen3-embedding:0.6b".into(),
            llm_model: "gemma4:e4b".into(),
            embed_dim: 1024,
            top_k_retrieval: 5,
        }
    }
}
