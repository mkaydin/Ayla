// ── Conversation store ─────────────────────────────────────
// Layer 2: Full conversation history in SurrealDB.

use crate::db::surreal::SurrealDB;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub session_id: String,
    pub turn: u32,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

pub struct ConversationStore {
    db: Arc<SurrealDB>,
}

impl ConversationStore {
    pub fn new(db: Arc<SurrealDB>) -> Self {
        Self { db }
    }

    pub async fn init_table(&self) -> Result<(), String> {
        self.db.ensure_table("conversation").await
    }

    pub async fn save_turn(&self, session_id: &str, role: &str, content: &str) -> Result<(), String> {
        // Get next turn number for this session (parameterized — issue R3).
        let mut qvars = serde_json::Map::new();
        qvars.insert("sid".into(), serde_json::Value::String(session_id.to_string()));
        let turn: u32 = match self.db
            .query_with_vars("SELECT count() FROM conversation WHERE session_id = $sid GROUP ALL;", qvars)
            .await
        {
            Ok(rows) => {
                rows.first()
                    .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
                    .unwrap_or(0) as u32
            }
            Err(_) => 0,
        };

        let record = ConversationRecord {
            session_id: session_id.to_string(),
            turn,
            role: role.to_string(),
            content: content.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        self.db.insert("conversation", record).await
    }

    pub async fn get_session(&self, session_id: &str) -> Result<Vec<ConversationRecord>, String> {
        let mut qvars = serde_json::Map::new();
        qvars.insert("sid".into(), serde_json::Value::String(session_id.to_string()));
        self.db
            .query_with_vars("SELECT * FROM conversation WHERE session_id = $sid ORDER BY turn ASC;", qvars)
            .await
            .and_then(|rows| {
                rows.into_iter()
                    .map(|v| serde_json::from_value::<ConversationRecord>(v).map_err(|e| e.to_string()))
                    .collect()
            })
    }

    pub async fn get_recent_context(&self, session_id: &str, n: u32) -> Result<String, String> {
        let records = self.get_session(session_id).await?;
        let recent: Vec<String> = records
            .into_iter()
            .rev()
            .take(n as usize)
            .rev()
            .map(|r| format!("{}: {}", r.role, r.content))
            .collect();
        Ok(recent.join("\n"))
    }
}
