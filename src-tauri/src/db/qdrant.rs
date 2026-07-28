// ── Qdrant client wrapper ───────────────────────────────────

use qdrant_client::qdrant::{
    value, CreateCollectionBuilder, Distance, PointId, PointStruct, ScoredPoint,
    SearchPointsBuilder, UpsertPointsBuilder, Value, VectorParamsBuilder,
};
use qdrant_client::{Qdrant, QdrantError};
use std::collections::HashMap;

pub struct QdrantDB {
    client: Qdrant,
}

impl QdrantDB {
    pub async fn connect(url: &str) -> Result<Self, String> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e: QdrantError| format!("Qdrant connect: {}", e))?;
        Ok(Self { client })
    }

    pub async fn ensure_collection(&self, name: &str, dim: u64) -> Result<(), String> {
        let exists = self.client
            .collection_exists(name)
            .await
            .map_err(|e| e.to_string())?;
        if exists { return Ok(()); }

        self.client
            .create_collection(
                CreateCollectionBuilder::new(name)
                    .vectors_config(VectorParamsBuilder::new(dim, Distance::Cosine)),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn upsert(&self, collection: &str, id: u64, vector: Vec<f32>, payload_text: &str) -> Result<(), String> {
        let mut payload = HashMap::new();
        payload.insert("text".to_string(), Value { kind: Some(value::Kind::StringValue(payload_text.to_string())) });

        let point = PointStruct {
            id: Some(PointId { point_id_options: Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(id)) }),
            vectors: Some(qdrant_client::qdrant::Vectors {
                vectors_options: Some(qdrant_client::qdrant::vectors::VectorsOptions::Vector(
                    #[allow(deprecated)]
                    qdrant_client::qdrant::Vector { data: vector, ..Default::default() }
                )),
            }),
            payload,
        };

        self.client
            .upsert_points(UpsertPointsBuilder::new(collection.to_string(), vec![point]))
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn search(&self, collection: &str, vector: Vec<f32>, limit: u64) -> Result<Vec<(u64, f32, String)>, String> {
        let response = self.client
            .search_points(
                SearchPointsBuilder::new(collection, vector, limit).with_payload(true),
            )
            .await
            .map_err(|e| e.to_string())?;

        let results: Vec<(u64, f32, String)> = response.result.into_iter().map(|p: ScoredPoint| {
            let text = p.payload
                .get("text")
                .and_then(|v| v.kind.as_ref())
                .and_then(|k| match k { value::Kind::StringValue(s) => Some(s.clone()), _ => None })
                .unwrap_or_default();
            let id = p.id.and_then(|pid| match pid.point_id_options {
                Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(n)) => Some(n),
                _ => None,
            }).unwrap_or(0);
            (id, p.score, text)
        }).collect();

        Ok(results)
    }
}
