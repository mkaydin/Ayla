// ── Episodic store ─────────────────────────────────────────
// Layer 3: Important events in SurrealDB + vectors in Qdrant.

use crate::db::surreal::SurrealDB;
use crate::db::qdrant::QdrantDB;
use crate::memory::embeddings::Embedder;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_POINT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: Option<String>,
    pub summary: String,
    pub importance: f32,
    pub entities: Vec<String>,
    pub timestamp: String,
    pub qdrant_point_id: u64,
}

pub struct EpisodeStore {
    db: Arc<SurrealDB>,
    qdrant: Arc<QdrantDB>,
    embedder: Embedder,
}

impl EpisodeStore {
    pub fn new(db: Arc<SurrealDB>, qdrant: Arc<QdrantDB>, embedder: Embedder) -> Self {
        Self { db, qdrant, embedder }
    }

    pub async fn init_table(&self) -> Result<(), String> {
        self.db.ensure_table("episode").await?;
        self.qdrant.ensure_collection("episodes", 1024).await
    }

    pub async fn store(&self, summary: &str, importance: f32, entities: &[String]) -> Result<(), String> {
        // Embed the summary
        let vector = self.embedder.embed(summary).await?;

        // Store in Qdrant
        let point_id = NEXT_POINT_ID.fetch_add(1, Ordering::SeqCst);
        self.qdrant.upsert("episodes", point_id, vector, summary).await?;

        // Store in SurrealDB
        let episode = Episode {
            id: None,
            summary: summary.to_string(),
            importance,
            entities: entities.to_vec(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            qdrant_point_id: point_id,
        };
        self.db.insert("episode", episode).await
    }

    pub async fn search_similar(&self, query: &str, limit: u64) -> Result<Vec<Episode>, String> {
        let vector = self.embedder.embed(query).await?;
        let results = self.qdrant.search("episodes", vector, limit).await?;

        let point_ids: Vec<u64> = results.iter().map(|(id, _, _)| *id).collect();
        if point_ids.is_empty() { return Ok(vec![]); }

        let ids_str: Vec<String> = point_ids.iter().map(|id| id.to_string()).collect();
        let sql = format!(
            "SELECT * FROM episode WHERE qdrant_point_id IN [{}];",
            ids_str.join(",")
        );

        deserialize_episodes(&self.db, &sql).await
    }
}

async fn deserialize_episodes(db: &Arc<SurrealDB>, sql: &str) -> Result<Vec<Episode>, String> {
    db.query(sql).await.and_then(|rows| {
        rows.into_iter()
            .map(|v| serde_json::from_value::<Episode>(v).map_err(|e| e.to_string()))
            .collect()
    })
}
