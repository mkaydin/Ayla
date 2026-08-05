// ── Episodic store ─────────────────────────────────────────
// Layer 3: Important events in SurrealDB + vectors in Qdrant.

use crate::db::qdrant::QdrantDB;
use crate::db::surreal::SurrealDB;
use crate::memory::embeddings::Embedder;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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
    embed_dim: u64,
}

impl EpisodeStore {
    pub fn new(
        db: Arc<SurrealDB>,
        qdrant: Arc<QdrantDB>,
        embedder: Embedder,
        embed_dim: u64,
    ) -> Self {
        Self {
            db,
            qdrant,
            embedder,
            embed_dim,
        }
    }

    pub async fn init_table(&self) -> Result<(), String> {
        self.db.ensure_table("episode").await?;
        self.qdrant
            .ensure_collection("episodes", self.embed_dim)
            .await?;
        // Seed the in-memory point-ID counter from the highest existing episode ID so that
        // new episodes don't overwrite existing Qdrant points after a restart (issue R4).
        Self::seed_point_id(&self.db).await;
        Ok(())
    }

    async fn seed_point_id(db: &Arc<SurrealDB>) {
        if let Ok(rows) = db
            .query("SELECT math::max(qdrant_point_id) AS max_id FROM episode GROUP ALL;")
            .await
        {
            let max_id = rows
                .first()
                .and_then(|r| r.get("max_id"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if max_id > 0 {
                NEXT_POINT_ID.store(max_id + 1, Ordering::SeqCst);
                log::info!("Episode point ID counter seeded to {}", max_id + 1);
            }
        }
    }

    pub async fn store(
        &self,
        summary: &str,
        importance: f32,
        entities: &[String],
    ) -> Result<(), String> {
        // Embed the summary
        let vector = self.embedder.embed(summary).await?;

        // Store in Qdrant
        let point_id = NEXT_POINT_ID.fetch_add(1, Ordering::SeqCst);
        self.qdrant
            .upsert("episodes", point_id, vector, summary)
            .await?;

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
        if point_ids.is_empty() {
            return Ok(vec![]);
        }

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
