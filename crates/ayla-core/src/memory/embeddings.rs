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

        let resp = self
            .client
            .post(format!("{}/api/embed", self.config.ollama_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Embed request failed: {}", e))?;

        let resp = resp
            .error_for_status()
            .map_err(|e| format!("Embedding HTTP error: {e}"))?;
        let json: Value = resp.json().await.map_err(|e| e.to_string())?;

        let embeddings = json["embeddings"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_array())
            .ok_or("No embedding in response")?;

        let vector: Vec<f32> = embeddings
            .iter()
            .map(|v| {
                v.as_f64()
                    .map(|f| f as f32)
                    .filter(|f| f.is_finite())
                    .ok_or_else(|| {
                        "Embedding contains a non-finite or nonnumeric value".to_string()
                    })
            })
            .collect::<Result<_, _>>()?;
        if vector.len() != self.config.embed_dim {
            return Err(format!(
                "Embedding dimension mismatch: got {}, expected {}",
                vector.len(),
                self.config.embed_dim
            ));
        }
        Ok(vector)
    }
}

pub(crate) async fn ensure_embedding_profile(
    db: &crate::db::surreal::SurrealDB,
    config: &MemoryConfig,
) -> Result<(), String> {
    db.ensure_table("embedding_profile").await?;
    let stored = db.query("SELECT * FROM embedding_profile:active;").await?;
    if let Some(profile) = stored.first() {
        validate_embedding_profile(profile, &config.embed_model, "", config.embed_dim)?;
    }
    // An unavailable model is reported by embedding requests. Keep storage and
    // durable failed-approval receipts usable while Ollama is offline.
    let response = reqwest::Client::new()
        .get(format!("{}/api/tags", config.ollama_url))
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    let tags = match response {
        Ok(response) if response.status().is_success() => {
            response.json::<Value>().await.unwrap_or_default()
        }
        _ => Value::Null,
    };
    let digest = tags
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models.iter().find(|model| {
                model.get("name").and_then(Value::as_str) == Some(config.embed_model.as_str())
            })
        })
        .and_then(|model| model.get("digest"))
        .and_then(Value::as_str)
        .or_else(|| {
            stored
                .first()
                .and_then(|profile| profile.get("digest"))
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    if let Some(profile) = stored.first() {
        validate_embedding_profile(profile, &config.embed_model, digest, config.embed_dim)?;
    }
    let mut vars = serde_json::Map::new();
    vars.insert("model".into(), Value::String(config.embed_model.clone()));
    vars.insert("digest".into(), Value::String(digest.into()));
    vars.insert("dimensions".into(), Value::from(config.embed_dim));
    db.exec_with_vars("UPSERT embedding_profile:active SET model = $model, digest = $digest, dimensions = $dimensions RETURN NONE;", vars).await
}

fn validate_embedding_profile(
    profile: &Value,
    model: &str,
    digest: &str,
    dimensions: usize,
) -> Result<(), String> {
    let saved_digest = profile.get("digest").and_then(Value::as_str).unwrap_or("");
    if profile.get("model").and_then(Value::as_str) != Some(model)
        || profile.get("dimensions").and_then(Value::as_u64) != Some(dimensions as u64)
        || (!saved_digest.is_empty() && !digest.is_empty() && saved_digest != digest)
    {
        return Err("Embedding model identity, digest, or dimensions changed. Restore the recorded model, or use a fresh database and vector prefix and explicitly reapprove/reindex source inputs. Existing vectors cannot be reused with a different model.".into());
    }
    Ok(())
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn same_dimension_model_changes_require_explicit_reindexing() {
        let saved =
            serde_json::json!({"model": "embedding:v1", "digest": "original", "dimensions": 1024});
        assert!(validate_embedding_profile(&saved, "embedding:v1", "original", 1024).is_ok());
        assert!(validate_embedding_profile(&saved, "embedding:v2", "original", 1024).is_err());
        assert!(validate_embedding_profile(&saved, "embedding:v1", "changed", 1024).is_err());
        assert!(validate_embedding_profile(&saved, "embedding:v1", "original", 768).is_err());
    }
}
