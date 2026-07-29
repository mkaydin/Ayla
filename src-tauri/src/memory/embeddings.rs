// ── Ollama embedding client ────────────────────────────────
// Uses qwen3-embedding:0.6b (1024-dim vectors)

use crate::memory::config::MemoryConfig;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct Embedder {
    config: MemoryConfig,
    client: reqwest::Client,
}

impl Embedder {
    pub fn new(config: MemoryConfig) -> Self {
        // Shared, reused HTTP client with a timeout so a hung Ollama doesn't stall the
        // IPC forever (issues R13/R14 — was: a fresh Client::new() per call, no timeout).
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let body = serde_json::json!({
            "model": self.config.embed_model,
            "input": [text],
        });

        let resp = self.client
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
