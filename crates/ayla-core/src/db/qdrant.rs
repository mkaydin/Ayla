// ── Qdrant client wrapper ───────────────────────────────────

use qdrant_client::qdrant::{
    value, Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, PointId,
    PointStruct, ScoredPoint, SearchPointsBuilder, UpsertPointsBuilder, Value, VectorParamsBuilder,
};
use qdrant_client::{Qdrant, QdrantError};
use std::collections::HashMap;

pub struct QdrantDB {
    client: Qdrant,
    collection_prefix: String,
}

impl QdrantDB {
    pub async fn connect(url: &str) -> Result<Self, String> {
        Self::connect_with_prefix(
            url,
            &std::env::var("AYLA_QDRANT_COLLECTION_PREFIX").unwrap_or_default(),
        )
        .await
    }

    pub async fn connect_with_prefix(url: &str, prefix: &str) -> Result<Self, String> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e: QdrantError| format!("Qdrant connect: {}", e))?;
        let collection_prefix = prefix.to_string();
        if !collection_prefix
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(
                "Qdrant collection prefix must contain only letters, digits, and underscores"
                    .into(),
            );
        }
        Ok(Self {
            client,
            collection_prefix,
        })
    }

    fn collection(&self, name: &str) -> String {
        format!("{}{}", self.collection_prefix, name)
    }

    pub async fn ensure_collection(&self, name: &str, dim: u64) -> Result<(), String> {
        let name = self.collection(name);
        let exists = self
            .client
            .collection_exists(&name)
            .await
            .map_err(|e| e.to_string())?;
        if exists {
            let info = self
                .client
                .collection_info(&name)
                .await
                .map_err(|e| e.to_string())?;
            let params = info
                .result
                .and_then(|i| i.config)
                .and_then(|c| c.params)
                .and_then(|p| p.vectors_config)
                .and_then(|v| v.config);
            match params {
                Some(qdrant_client::qdrant::vectors_config::Config::Params(p)) if p.size == dim => return Ok(()),
                _ => return Err(format!("Collection {name} has incompatible vectors; expected {dim} dimensions. Reindex into a new collection.")),
            }
        }

        self.client
            .create_collection(
                CreateCollectionBuilder::new(&name)
                    .vectors_config(VectorParamsBuilder::new(dim, Distance::Cosine)),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn upsert(
        &self,
        collection: &str,
        id: u64,
        vector: Vec<f32>,
        payload_text: &str,
    ) -> Result<(), String> {
        let mut payload = HashMap::new();
        payload.insert(
            "text".to_string(),
            Value {
                kind: Some(value::Kind::StringValue(payload_text.to_string())),
            },
        );
        self.upsert_with_payload(collection, id, vector, payload)
            .await
    }

    /// Upsert a point with arbitrary payload. Caller builds the payload map.
    pub async fn upsert_with_payload(
        &self,
        collection: &str,
        id: u64,
        vector: Vec<f32>,
        payload: HashMap<String, Value>,
    ) -> Result<(), String> {
        let point = PointStruct {
            id: Some(PointId {
                point_id_options: Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(id)),
            }),
            vectors: Some(qdrant_client::qdrant::Vectors {
                vectors_options: Some(qdrant_client::qdrant::vectors::VectorsOptions::Vector(
                    #[allow(deprecated)]
                    qdrant_client::qdrant::Vector {
                        data: vector,
                        ..Default::default()
                    },
                )),
            }),
            payload,
        };

        self.client
            .upsert_points(
                UpsertPointsBuilder::new(self.collection(collection), vec![point]).wait(true),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delete all points in `collection` whose `field` equals `value`.
    pub async fn delete_by_field(
        &self,
        collection: &str,
        field: &str,
        value: &str,
    ) -> Result<(), String> {
        let filter = Filter::must([Condition::matches(field.to_string(), value.to_string())]);
        self.client
            .delete_points(
                DeletePointsBuilder::new(self.collection(collection))
                    .points(filter)
                    .wait(true),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn search_with_payload(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: u64,
    ) -> Result<Vec<ScoredPoint>, String> {
        self.client
            .search_points(
                SearchPointsBuilder::new(self.collection(collection), vector, limit)
                    .with_payload(true),
            )
            .await
            .map(|response| response.result)
            .map_err(|e| e.to_string())
    }

    pub async fn search(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: u64,
    ) -> Result<Vec<(u64, f32, String)>, String> {
        let response = self
            .client
            .search_points(
                SearchPointsBuilder::new(self.collection(collection), vector, limit)
                    .with_payload(true),
            )
            .await
            .map_err(|e| e.to_string())?;

        let results: Vec<(u64, f32, String)> = response
            .result
            .into_iter()
            .map(|p: ScoredPoint| {
                let text = p
                    .payload
                    .get("text")
                    .and_then(|v| v.kind.as_ref())
                    .and_then(|k| match k {
                        value::Kind::StringValue(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let id =
                    p.id.and_then(|pid| match pid.point_id_options {
                        Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(n)) => Some(n),
                        _ => None,
                    })
                    .unwrap_or(0);
                (id, p.score, text)
            })
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::QdrantDB;

    #[tokio::test]
    #[ignore = "requires the local Docker Qdrant service"]
    async fn local_server_is_client_compatible() {
        let db = QdrantDB::connect("http://127.0.0.1:6334")
            .await
            .expect("Qdrant client/server compatibility check failed");

        db.client
            .list_collections()
            .await
            .expect("Qdrant collections request failed");
    }
}
