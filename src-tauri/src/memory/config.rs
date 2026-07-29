// ── Memory configuration ────────────────────────────────────

#[derive(Clone)]
pub struct MemoryConfig {
    pub surrealdb_url: String,
    pub surrealdb_ns: String,
    pub surrealdb_db: String,
    pub surrealdb_user: String,
    pub surrealdb_pass: String,
    pub qdrant_url: String,
    pub ollama_url: String,
    pub embed_model: String,
    pub llm_model: String,
    pub embed_dim: usize,
    pub top_k_retrieval: usize,
}

fn env_or(default: &str, key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            surrealdb_url: env_or("http://127.0.0.1:8000", "SURREALDB_URL"),
            surrealdb_ns: env_or("alice", "SURREALDB_NS"),
            surrealdb_db: env_or("memory", "SURREALDB_DB"),
            // Credentials come from env vars (issue R5). Change the default in production via
            // a gitignored .env file; never commit real credentials.
            surrealdb_user: env_or("root", "SURREALDB_USER"),
            surrealdb_pass: env_or("root", "SURREALDB_PASS"),
            qdrant_url: env_or("http://127.0.0.1:6334", "QDRANT_URL"),
            ollama_url: env_or("http://127.0.0.1:11434", "OLLAMA_URL"),
            embed_model: env_or("qwen3-embedding:0.6b", "ALICE_EMBED_MODEL"),
            llm_model: env_or("gemma4:e4b", "ALICE_LLM_MODEL"),
            embed_dim: 1024,
            top_k_retrieval: 5,
        }
    }
}
