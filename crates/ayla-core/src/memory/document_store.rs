// ── Document store (Qdrant-based vector documents) ─────────
// Stores document chunks as 1024-dim vectors with metadata.

use crate::db::qdrant::QdrantDB;
use crate::memory::embeddings::Embedder;
use qdrant_client::qdrant::{value, Value};
use std::sync::Arc;

pub struct DocumentStore {
    qdrant: Arc<QdrantDB>,
    embedder: Embedder,
    embed_dim: u64,
    db: Option<Arc<crate::db::surreal::SurrealDB>>,
}

impl DocumentStore {
    pub fn new(qdrant: Arc<QdrantDB>, embedder: Embedder, embed_dim: u64) -> Self {
        Self {
            qdrant,
            embedder,
            embed_dim,
            db: None,
        }
    }

    pub fn with_database(mut self, db: Arc<crate::db::surreal::SurrealDB>) -> Self {
        self.db = Some(db);
        self
    }

    pub async fn init(&self) -> Result<(), String> {
        self.qdrant
            .ensure_collection("documents", self.embed_dim)
            .await
    }

    pub async fn store_chunks(
        &self,
        filename: &str,
        chunks: &[String],
    ) -> Result<Vec<u64>, String> {
        let mut ids = vec![];
        for (i, chunk) in chunks.iter().enumerate() {
            let vector = self.embedder.embed(chunk).await?;
            let id = Self::chunk_id(filename, i as u32);
            // Store filename + index in payload so we can filter-delete later and identify origin.
            let mut payload = std::collections::HashMap::new();
            payload.insert(
                "text".to_string(),
                Value {
                    kind: Some(value::Kind::StringValue(chunk.clone())),
                },
            );
            payload.insert(
                "filename".to_string(),
                Value {
                    kind: Some(value::Kind::StringValue(filename.to_string())),
                },
            );
            payload.insert(
                "chunk_index".to_string(),
                Value {
                    kind: Some(value::Kind::IntegerValue(i as i64)),
                },
            );
            self.qdrant
                .upsert_with_payload("documents", id, vector, payload)
                .await?;
            ids.push(id);
            log::info!(
                "DocumentStore: stored chunk {}/{} for '{}' ({} chars)",
                i + 1,
                chunks.len(),
                filename,
                chunk.len()
            );
        }
        Ok(ids)
    }

    pub async fn search(&self, query: &str, limit: u64) -> Result<Vec<(String, f32)>, String> {
        let vector = self.embedder.embed(query).await?;
        let results = self
            .qdrant
            .search_with_payload("documents", vector, limit.saturating_mul(4).min(80))
            .await?;
        let mut blocked = std::collections::HashSet::new();
        if let Some(db) = &self.db {
            for query in [
                "SELECT name FROM kb_file WHERE status IN ['failed', 'deleting'];",
                "SELECT name FROM knowledge_operation WHERE status != 'committed';",
            ] {
                for row in db.query(query).await? {
                    if let Some(name) = row.get("name").and_then(|v| v.as_str()) {
                        blocked.insert(name.to_string());
                    }
                }
            }
        }
        Ok(results
            .into_iter()
            .filter_map(|point| {
                let string = |key: &str| {
                    point
                        .payload
                        .get(key)
                        .and_then(|v| v.kind.as_ref())
                        .and_then(|v| {
                            if let value::Kind::StringValue(s) = v {
                                Some(s.as_str())
                            } else {
                                None
                            }
                        })
                };
                let filename = string("filename")?;
                if blocked.contains(filename) {
                    return None;
                }
                Some((
                    format!("[Document source: {filename}] {}", string("text")?),
                    point.score,
                ))
            })
            .take(limit as usize)
            .collect())
    }

    pub async fn delete_by_filename(&self, filename: &str) -> Result<(), String> {
        // Actually delete vectors by filename payload filter (was a no-op loop — issue R18).
        self.qdrant
            .delete_by_field("documents", "filename", filename)
            .await?;
        log::info!("DocumentStore: deleted vectors for '{}'", filename);
        Ok(())
    }

    /// Deterministic chunk ID derived from a stable FNV-1a hash of the filename, so the
    /// same file re-ingested after restart overwrites the same points instead of creating
    /// duplicates (was: std DefaultHasher which is randomized per-process — issue R19).
    pub fn chunk_id(filename: &str, idx: u32) -> u64 {
        fnv1a_64(format!("{filename}\0{idx}").as_bytes())
    }
}

/// 64-bit FNV-1a hash (deterministic, no extra deps).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}
