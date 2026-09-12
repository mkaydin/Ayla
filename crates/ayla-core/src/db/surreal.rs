// ── SurrealDB client wrapper (direct HTTP) ──────────────────

use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

/// SurrealDB 2.1.x can persist a NUL character but then panic while serializing
/// that record for an HTTP response. Web pages, OCR output, and model output are
/// all untrusted text sources, so remove NULs recursively at the database
/// boundary rather than relying on every caller to remember to clean them.
pub(crate) fn sanitize_value_for_storage(value: &mut Value) {
    match value {
        Value::String(text) => {
            if text.contains('\0') {
                *text = text.replace('\0', " ");
            }
        }
        Value::Array(values) => {
            for value in values {
                sanitize_value_for_storage(value);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                sanitize_value_for_storage(value);
            }
        }
        _ => {}
    }
}

pub struct SurrealDB {
    url: String,
    ns: String,
    db: String,
    user: String,
    pass: String,
    // Shared, reused HTTP client with timeout (issues R13/R14).
    client: reqwest::Client,
}

impl SurrealDB {
    pub async fn connect(
        url: &str,
        ns: &str,
        db_name: &str,
        user: &str,
        pass: &str,
    ) -> Result<Self, String> {
        if ![ns, db_name].iter().all(|name| {
            !name.is_empty()
                && name.len() <= 128
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }) {
            return Err(
                "SurrealDB namespace/database must contain only letters, digits, and underscores"
                    .into(),
            );
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| format!("reqwest build: {}", e))?;
        let instance = Self {
            url: url.trim_end_matches('/').to_string(),
            ns: ns.to_string(),
            db: db_name.to_string(),
            user: user.to_string(),
            pass: pass.to_string(),
            client,
        };
        instance.bootstrap().await?;
        Ok(instance)
    }

    async fn bootstrap(&self) -> Result<(), String> {
        let sql = format!("USE NS {} DB {}; RETURN 1;", self.ns, self.db);
        let resp = self
            .client
            .post(format!("{}/sql", self.url))
            .basic_auth(&self.user, Some(&self.pass))
            .header("Accept", "application/json")
            .body(sql)
            .send()
            .await
            .map_err(|e| format!("SurrealDB bootstrap: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("SurrealDB bootstrap: HTTP {}", resp.status()));
        }
        self.check_response(
            &resp
                .json()
                .await
                .map_err(|e| format!("SurrealDB bootstrap parse: {}", e))?,
        )?;
        Ok(())
    }

    fn check_response(&self, body: &Value) -> Result<(), String> {
        if let Value::Array(arr) = body {
            for item in arr {
                self.check_single(item)?;
            }
            Ok(())
        } else {
            self.check_single(body)
        }
    }

    fn check_single(&self, item: &Value) -> Result<(), String> {
        match item.get("status").and_then(|v| v.as_str()) {
            Some("OK") => Ok(()),
            Some("ERR") => {
                let msg = item
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                Err(format!("SurrealDB: {}", msg))
            }
            _ => Ok(()),
        }
    }

    pub async fn ensure_table(&self, name: &str) -> Result<(), String> {
        let sql = format!("DEFINE TABLE IF NOT EXISTS {};", name);
        self.query_raw(&sql).await?;
        Ok(())
    }

    pub async fn insert<T: Serialize>(&self, table: &str, record: T) -> Result<(), String> {
        let mut json = serde_json::to_value(&record).map_err(|e| e.to_string())?;
        sanitize_value_for_storage(&mut json);
        // INSERT returns the complete record by default. Large document/search
        // records do not need to make a second trip over HTTP, and returning a
        // malformed legacy string can crash SurrealDB 2.1's response serializer.
        let sql = format!("INSERT INTO {} {} RETURN NONE;", table, json);
        self.query_raw(&sql).await?;
        Ok(())
    }

    pub async fn query(&self, sql: &str) -> Result<Vec<Value>, String> {
        let body: Value = self.query_raw(sql).await?;
        let results: Vec<Value> = match &body {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|r| r.get("result"))
                .filter_map(|v| match v {
                    Value::Array(arr) => Some(arr.clone()),
                    _ => None,
                })
                .flatten()
                .collect(),
            _ => vec![],
        };
        Ok(results)
    }

    /// Execute a parameterized query using SurrealDB `$var` placeholders.
    ///
    /// `sql` should reference variables as `$name`; `vars` maps `name -> value`.
    /// This is the SQL-injection-safe path (issue R3) — never interpolate user/frontend
    /// input into `sql` directly.
    ///
    /// Implementation note: SurrealDB's HTTP `POST /sql` endpoint takes SurrealQL TEXT as
    /// the body (NOT a JSON `{sql,vars}` object), and bound parameters are normally passed
    /// as URL query params. Rather than encode values as URL query strings (whose parsing
    /// rules for strings/quotes are awkward), we prepend `LET $name = <json>;` statements.
    /// JSON-encoding the values makes this injection-safe: a string like `O'Brien` becomes
    /// the SurrealQL literal `"O'Brien"`, and embedded quotes/backslashes are escaped by
    /// `serde_json`. Numbers/bools/null serialize to valid SurrealQL literals directly.
    pub async fn query_with_vars(
        &self,
        sql: &str,
        vars: serde_json::Map<String, Value>,
    ) -> Result<Vec<Value>, String> {
        let mut full = String::new();
        for (name, val) in vars {
            // Validate the variable name ($-prefixed, alphanumeric + underscore only) so a
            // crafted name can't break out of the LET statement.
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("Invalid query variable name: {}", name));
            }
            let mut val = val;
            sanitize_value_for_storage(&mut val);
            let json = serde_json::to_string(&val).map_err(|e| format!("var serialize: {}", e))?;
            full.push_str(&format!("LET ${} = {}; ", name, json));
        }
        full.push_str(sql);
        let body: Value = self.query_raw(&full).await?;
        let results: Vec<Value> = match &body {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|r| r.get("result"))
                .filter_map(|v| match v {
                    Value::Array(arr) => Some(arr.clone()),
                    _ => None,
                })
                .flatten()
                .collect(),
            _ => vec![],
        };
        Ok(results)
    }

    /// Convenience: run a statement that returns no rows (INSERT/UPDATE/DELETE/DEFINE) with
    /// bound variables. Returns Ok(()) on a successful status.
    pub async fn exec_with_vars(
        &self,
        sql: &str,
        vars: serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        self.query_with_vars(sql, vars).await.map(|_| ())
    }

    async fn query_raw(&self, sql: &str) -> Result<Value, String> {
        // Also guard hand-built maintenance queries. Structured records and
        // variables are sanitized above; this covers any literal raw NUL.
        let full_sql = format!("USE NS {} DB {}; {}", self.ns, self.db, sql).replace('\0', " ");
        let resp = self
            .client
            .post(format!("{}/sql", self.url))
            .basic_auth(&self.user, Some(&self.pass))
            .header("Accept", "application/json")
            .body(full_sql)
            .send()
            .await
            .map_err(|e| {
                let kind = if e.is_timeout() {
                    "timeout"
                } else if e.is_connect() {
                    "connection"
                } else if e.is_request() {
                    "request"
                } else if e.is_body() {
                    "body"
                } else {
                    "transport"
                };
                format!("SurrealDB {} error: {}", kind, e)
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "SurrealDB HTTP {}: {}",
                status,
                crate::memory::safe_truncate(&body, 600)
            ));
        }

        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("SurrealDB parse: {}", e))?;
        self.check_response(&body)?;
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_value_for_storage;

    #[test]
    fn storage_sanitizer_removes_nested_nul_characters() {
        let mut value = serde_json::json!({
            "text": "alpha\0beta",
            "nested": [{"value": "\0gamma"}],
            "untouched": "line one\nline two"
        });
        sanitize_value_for_storage(&mut value);

        assert_eq!(value["text"], "alpha beta");
        assert_eq!(value["nested"][0]["value"], " gamma");
        assert_eq!(value["untouched"], "line one\nline two");
        assert!(!value.to_string().contains("\\u0000"));
    }
}
