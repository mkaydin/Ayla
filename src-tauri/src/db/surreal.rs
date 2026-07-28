// ── SurrealDB client wrapper (direct HTTP) ──────────────────

use serde::Serialize;
use serde_json::Value;

pub struct SurrealDB {
    url: String,
    ns: String,
    db: String,
    user: String,
    pass: String,
}

impl SurrealDB {
    pub async fn connect(url: &str, ns: &str, db_name: &str, user: &str, pass: &str) -> Result<Self, String> {
        let client = Self {
            url: url.trim_end_matches('/').to_string(),
            ns: ns.to_string(),
            db: db_name.to_string(),
            user: user.to_string(),
            pass: pass.to_string(),
        };
        let _ = client.query_raw("RETURN 1;").await?;
        Ok(client)
    }

    pub async fn ensure_table(&self, name: &str) -> Result<(), String> {
        let sql = format!("DEFINE TABLE IF NOT EXISTS {};", name);
        self.query_raw(&sql).await?;
        Ok(())
    }

    pub async fn insert<T: Serialize>(&self, table: &str, record: T) -> Result<(), String> {
        let json = serde_json::to_value(&record).map_err(|e| e.to_string())?;
        let sql = format!("INSERT INTO {} {};", table, json);
        self.query_raw(&sql).await?;
        Ok(())
    }

    pub async fn query(&self, sql: &str) -> Result<Vec<Value>, String> {
        let body: Value = self.query_raw(sql).await?;
        let results: Vec<Value> = match body {
            Value::Array(arr) => arr.into_iter().flat_map(|r| {
                match r.get("result") {
                    Some(Value::Array(arr)) => arr.clone(),
                    Some(v) => vec![v.clone()],
                    None => vec![],
                }
            }).collect(),
            _ => vec![],
        };
        Ok(results)
    }

    async fn query_raw(&self, sql: &str) -> Result<Value, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(&format!("{}/sql", self.url))
            .basic_auth(&self.user, Some(&self.pass))
            .header("NS", &self.ns)
            .header("DB", &self.db)
            .header("Accept", "application/json")
            .body(sql.to_string())
            .send()
            .await
            .map_err(|e| format!("SurrealDB: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("SurrealDB HTTP {}: {}", status, &body[..body.len().min(200)]));
        }

        resp.json().await.map_err(|e| format!("SurrealDB parse: {}", e))
    }
}
