//! Dedicated vector storage for code-analysis entities and generated documents.

use crate::db::qdrant::QdrantDB;
use crate::memory::embeddings::Embedder;
use crate::memory::safe_truncate;
use ayla_code_analysis::GeneratedDocuments;
use qdrant_client::qdrant::{value, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const CODE_ANALYSIS_COLLECTION: &str = "code_analysis";
const MAX_EMBED_TEXT_BYTES: usize = 8 * 1024;
const MAX_DOCUMENT_CHUNKS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeAnalysisVectorReceipt {
    pub entity_vectors: usize,
    pub document_vectors: usize,
}

pub struct CodeAnalysisVectorStore {
    qdrant: Arc<QdrantDB>,
    embedder: Embedder,
    embed_dim: u64,
}

impl CodeAnalysisVectorStore {
    pub fn new(qdrant: Arc<QdrantDB>, embedder: Embedder, embed_dim: u64) -> Self {
        Self {
            qdrant,
            embedder,
            embed_dim,
        }
    }

    pub async fn init(&self) -> Result<(), String> {
        self.qdrant
            .ensure_collection(CODE_ANALYSIS_COLLECTION, self.embed_dim)
            .await
    }

    pub async fn index_run(
        &self,
        operation_id: &str,
        documents: &GeneratedDocuments,
    ) -> Result<CodeAnalysisVectorReceipt, String> {
        self.delete_run(operation_id).await?;
        let result = self.index_run_inner(operation_id, documents).await;
        if result.is_err() {
            let _ = self.delete_run(operation_id).await;
        }
        result
    }

    async fn index_run_inner(
        &self,
        operation_id: &str,
        documents: &GeneratedDocuments,
    ) -> Result<CodeAnalysisVectorReceipt, String> {
        let mut entity_vectors = 0;
        for entity in &documents.entities {
            let fallback = format!(
                "{} {}. {} Location: {}",
                entity.label,
                entity.name,
                entity.description.as_deref().unwrap_or(""),
                entity.source_location.as_deref().unwrap_or("unknown")
            );
            let text = entity.embedding_text.as_deref().unwrap_or(&fallback);
            if text.trim().is_empty() {
                continue;
            }
            let text = safe_truncate(text, MAX_EMBED_TEXT_BYTES);
            let vector = self.embed_checked(text).await?;
            let mut payload = base_payload(operation_id, documents, "entity", text);
            payload.insert("uid".into(), string_value(&entity.uid));
            payload.insert("label".into(), string_value(&entity.label));
            payload.insert("name".into(), string_value(&entity.name));
            payload.insert(
                "source_location".into(),
                string_value(entity.source_location.as_deref().unwrap_or("")),
            );
            self.qdrant
                .upsert_with_payload(
                    CODE_ANALYSIS_COLLECTION,
                    stable_point_id(operation_id, "entity", &entity.uid),
                    vector,
                    payload,
                )
                .await?;
            entity_vectors += 1;
        }

        let mut document_vectors = 0;
        for (kind, markdown) in [
            ("documentation", documents.human_markdown.as_str()),
            ("knowledge_base", documents.knowledge_base_markdown.as_str()),
        ] {
            for (index, chunk) in bounded_chunks(markdown).into_iter().enumerate() {
                let vector = self.embed_checked(chunk).await?;
                let mut payload = base_payload(operation_id, documents, kind, chunk);
                payload.insert("chunk_index".into(), integer_value(index as i64));
                self.qdrant
                    .upsert_with_payload(
                        CODE_ANALYSIS_COLLECTION,
                        stable_point_id(operation_id, kind, &index.to_string()),
                        vector,
                        payload,
                    )
                    .await?;
                document_vectors += 1;
            }
        }
        Ok(CodeAnalysisVectorReceipt {
            entity_vectors,
            document_vectors,
        })
    }

    async fn embed_checked(&self, text: &str) -> Result<Vec<f32>, String> {
        let vector = self.embedder.embed(text).await?;
        if vector.len() != self.embed_dim as usize {
            return Err(format!(
                "Embedding model returned {} dimensions; code-analysis collection expects {}",
                vector.len(),
                self.embed_dim
            ));
        }
        Ok(vector)
    }

    /// Only fully approved runs contribute context. Preview/failed/superseded vectors are excluded.
    pub async fn search(&self, query: &str, limit: u64) -> Result<Vec<(String, f32)>, String> {
        let vector = self.embed_checked(query).await?;
        let points = self
            .qdrant
            .search_with_payload(
                CODE_ANALYSIS_COLLECTION,
                vector,
                limit.saturating_mul(4).min(80),
            )
            .await?;
        let mut results = Vec::new();
        for point in points {
            let string = |key: &str| {
                point
                    .payload
                    .get(key)
                    .and_then(|v| v.kind.as_ref())
                    .and_then(|kind| {
                        if let value::Kind::StringValue(s) = kind {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
            };
            let Some(operation) = string("operation_id") else {
                continue;
            };
            let Ok(run) = crate::artifacts::load_code_analysis_run(operation) else {
                continue;
            };
            if run.status != "approved" || !run.graph_approved || !run.vectors_indexed {
                continue;
            }
            if let Some(text) = string("text") {
                results.push((
                    format!(
                        "[Code source: {}; run: {}; location: {}] {}",
                        string("project_name").unwrap_or("unknown"),
                        operation,
                        string("source_location").unwrap_or("documentation"),
                        text
                    ),
                    point.score,
                ));
            }
            if results.len() >= limit as usize {
                break;
            }
        }
        Ok(results)
    }

    pub async fn delete_run(&self, operation_id: &str) -> Result<(), String> {
        self.qdrant
            .delete_by_field(CODE_ANALYSIS_COLLECTION, "operation_id", operation_id)
            .await
    }
}

fn base_payload(
    operation_id: &str,
    documents: &GeneratedDocuments,
    kind: &str,
    text: &str,
) -> HashMap<String, Value> {
    HashMap::from([
        ("operation_id".into(), string_value(operation_id)),
        (
            "repository_uid".into(),
            string_value(&documents.repository_uid),
        ),
        ("project_name".into(), string_value(&documents.project_name)),
        ("record_kind".into(), string_value(kind)),
        ("text".into(), string_value(text)),
    ])
}

fn string_value(value: &str) -> Value {
    Value {
        kind: Some(value::Kind::StringValue(value.to_string())),
    }
}

fn integer_value(value: i64) -> Value {
    Value {
        kind: Some(value::Kind::IntegerValue(value)),
    }
}

fn bounded_chunks(markdown: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut remaining = markdown.trim();
    while !remaining.is_empty() && chunks.len() < MAX_DOCUMENT_CHUNKS {
        let candidate = safe_truncate(remaining, MAX_EMBED_TEXT_BYTES);
        let end = candidate
            .rfind("\n\n")
            .filter(|end| *end >= MAX_EMBED_TEXT_BYTES / 3)
            .unwrap_or(candidate.len());
        let (chunk, rest) = remaining.split_at(end);
        if !chunk.trim().is_empty() {
            chunks.push(chunk.trim());
        }
        remaining = rest.trim_start();
    }
    chunks
}

fn stable_point_id(operation_id: &str, kind: &str, identity: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in operation_id
        .bytes()
        .chain([0])
        .chain(kind.bytes())
        .chain([0])
        .chain(identity.bytes())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_are_bounded_and_utf8_safe() {
        let text = "🦀 paragraph\n\n".repeat(2_000);
        let chunks = bounded_chunks(&text);
        assert!(!chunks.is_empty());
        assert!(chunks.len() <= MAX_DOCUMENT_CHUNKS);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.len() <= MAX_EMBED_TEXT_BYTES));
    }

    #[test]
    fn point_ids_include_run_and_kind() {
        assert_ne!(
            stable_point_id("run-a", "entity", "x"),
            stable_point_id("run-b", "entity", "x")
        );
        assert_ne!(
            stable_point_id("run-a", "entity", "x"),
            stable_point_id("run-a", "documentation", "x")
        );
    }
}
