// ── Semantic store ─────────────────────────────────────────
// Layer 4: Facts about the user in SurrealDB.

use crate::db::surreal::SurrealDB;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    pub source: String,
    pub first_seen: String,
    pub last_seen: String,
    pub count: u32,
}

#[derive(Clone)]
pub struct SemanticStore {
    db: Arc<SurrealDB>,
}

impl SemanticStore {
    pub fn new(db: Arc<SurrealDB>) -> Self {
        Self { db }
    }

    pub async fn init_table(&self) -> Result<(), String> {
        self.db.ensure_table("fact").await
    }

    pub async fn upsert_fact(&self, subject: &str, predicate: &str, object: &str, confidence: f32) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        let sql = format!(
            "SELECT * FROM fact WHERE subject = '{}' AND predicate = '{}' AND object = '{}';",
            subject, predicate, object
        );
        let existing = self.db.query(&sql).await.ok().unwrap_or_default();

        if let Some(first) = existing.first() {
            let id = first.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let count = first.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as u32 + 1;
            let new_conf = ((confidence + confidence) / 2.0).min(1.0);

            let update_sql = format!(
                "UPDATE {} SET confidence = {}, count = {}, last_seen = '{}';",
                id, new_conf, count, now
            );
            self.db.query(&update_sql).await?;
        } else {
            let record = Fact {
                id: None,
                subject: subject.to_string(),
                predicate: predicate.to_string(),
                object: object.to_string(),
                confidence,
                source: "conversation".into(),
                first_seen: now.clone(),
                last_seen: now,
                count: 1,
            };
            self.db.insert("fact", record).await?;
        }
        Ok(())
    }

    pub async fn get_facts_about(&self, subject: &str) -> Result<Vec<Fact>, String> {
        let sql = format!(
            "SELECT * FROM fact WHERE subject = '{}' ORDER BY confidence DESC;",
            subject
        );
        deserialize_rows(&self.db, &sql).await
    }

    pub async fn search_facts(&self, query: &str) -> Result<Vec<Fact>, String> {
        let sql = format!(
            "SELECT * FROM fact WHERE subject ~ '{}' OR predicate ~ '{}' OR object ~ '{}' LIMIT 20;",
            query, query, query
        );
        deserialize_rows(&self.db, &sql).await
    }

    pub async fn get_all_facts(&self) -> Result<Vec<Fact>, String> {
        let sql = "SELECT * FROM fact;";
        deserialize_rows(&self.db, sql).await
    }

    pub async fn delete_fact(&self, id: &str) -> Result<(), String> {
        let sql = format!("DELETE {};", id);
        self.db.query(&sql).await?;
        Ok(())
    }

    pub async fn delete_by_predicate_object(&self, predicate: &str, object: &str) -> Result<(), String> {
        let sql = format!(
            "DELETE fact WHERE predicate = '{}' AND object = '{}';",
            predicate, object
        );
        self.db.query(&sql).await?;
        Ok(())
    }
}

async fn deserialize_rows(db: &Arc<SurrealDB>, sql: &str) -> Result<Vec<Fact>, String> {
    db.query(sql).await.and_then(|rows| {
        rows.into_iter()
            .map(|v| serde_json::from_value::<Fact>(v).map_err(|e| e.to_string()))
            .collect()
    })
}
