// ── Semantic store ─────────────────────────────────────────
// Layer 4: Facts about the user in SurrealDB.

use crate::db::surreal::SurrealDB;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
        // Parameterized to prevent SQL injection via LLM-extracted strings (issue R3).
        let mut vars = serde_json::Map::new();
        vars.insert("subject".into(), Value::String(subject.to_string()));
        vars.insert("predicate".into(), Value::String(predicate.to_string()));
        vars.insert("object".into(), Value::String(object.to_string()));
        let existing = self.db
            .query_with_vars("SELECT * FROM fact WHERE subject = $subject AND predicate = $predicate AND object = $object;", vars)
            .await
            .unwrap_or_default();

        if let Some(first) = existing.first() {
            let id = first.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let count = first.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as u32 + 1;
            // Average with the EXISTING confidence (was `confidence + confidence`, a no-op — issue R17).
            let old_conf = first.get("confidence").and_then(|v| v.as_f64()).unwrap_or(confidence as f64) as f32;
            let new_conf = ((old_conf + confidence) / 2.0).min(1.0);

            let mut upd = serde_json::Map::new();
            upd.insert("id".into(), Value::String(id.to_string()));
            upd.insert("confidence".into(), Value::from(new_conf as f64));
            upd.insert("count".into(), Value::from(count));
            upd.insert("last_seen".into(), Value::String(now));
            self.db.exec_with_vars("UPDATE type::thing($id) SET confidence = $confidence, count = $count, last_seen = $last_seen;", upd).await?;
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
        let mut vars = serde_json::Map::new();
        vars.insert("subject".into(), Value::String(subject.to_string()));
        deserialize_rows_var(&self.db, "SELECT * FROM fact WHERE subject = $subject ORDER BY confidence DESC;", vars).await
    }

    pub async fn search_facts(&self, query: &str) -> Result<Vec<Fact>, String> {
        let mut vars = serde_json::Map::new();
        vars.insert("q".into(), Value::String(query.to_string()));
        deserialize_rows_var(&self.db, "SELECT * FROM fact WHERE subject ~ $q OR predicate ~ $q OR object ~ $q LIMIT 20;", vars).await
    }

    pub async fn get_all_facts(&self) -> Result<Vec<Fact>, String> {
        let sql = "SELECT * FROM fact;";
        deserialize_rows(&self.db, sql).await
    }

    pub async fn delete_fact(&self, id: &str) -> Result<(), String> {
        let mut vars = serde_json::Map::new();
        vars.insert("id".into(), Value::String(id.to_string()));
        self.db.exec_with_vars("DELETE type::thing($id);", vars).await?;
        Ok(())
    }

    pub async fn delete_by_predicate_object(&self, predicate: &str, object: &str) -> Result<(), String> {
        let mut vars = serde_json::Map::new();
        vars.insert("predicate".into(), Value::String(predicate.to_string()));
        vars.insert("object".into(), Value::String(object.to_string()));
        self.db.exec_with_vars("DELETE fact WHERE predicate = $predicate AND object = $object;", vars).await?;
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

async fn deserialize_rows_var(db: &Arc<SurrealDB>, sql: &str, vars: serde_json::Map<String, Value>) -> Result<Vec<Fact>, String> {
    db.query_with_vars(sql, vars).await.and_then(|rows| {
        rows.into_iter()
            .map(|v| serde_json::from_value::<Fact>(v).map_err(|e| e.to_string()))
            .collect()
    })
}
