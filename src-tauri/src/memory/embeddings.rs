// ── Ollama embedding client ────────────────────────────────
// Uses qwen3-embedding:0.6b (1024-dim vectors)

use crate::memory::config::MemoryConfig;
use serde_json::Value;

#[derive(Clone)]
pub struct Embedder {
    config: MemoryConfig,
}

impl Embedder {
    pub fn new(config: MemoryConfig) -> Self {
        Self { config }
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": self.config.embed_model,
            "input": [text],
        });

        let resp = client
            .post(&format!("{}/api/embed", self.config.ollama_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Embed request failed: {}", e))?;

        let json: Value = resp.json().await.map_err(|e| e.to_string())?;

        let embeddings = json["embeddings"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_array())
            .ok_or("No embedding in response")?;

        Ok(embeddings.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect())
    }
}
