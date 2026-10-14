//! Backend history operations.

use super::*;

impl BackendApi<'_> {
    pub async fn load_chat_sessions(&self) -> Result<Vec<ChatSessionRecord>, BackendError> {
        if !self.manager.is_db_ready() {
            return Ok(Vec::new());
        }

        let db = self.connect_database().await?;
        db.ensure_table("chat_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("chat_message")
            .await
            .map_err(BackendError::from)?;
        let sessions = db
            .query("SELECT * FROM chat_session ORDER BY updated_at DESC;")
            .await
            .map_err(BackendError::from)?;
        let mut result = Vec::with_capacity(sessions.len());
        for session in sessions {
            let session_id = string_field(&session, "session_id");
            if session_id.is_empty() {
                continue;
            }
            let mut variables = serde_json::Map::new();
            variables.insert("sid".into(), Value::String(session_id.clone()));
            let messages = db
                .query_with_vars(
                    "SELECT * FROM chat_message WHERE session_id = $sid ORDER BY timestamp ASC;",
                    variables,
                )
                .await
                .map_err(BackendError::from)?
                .iter()
                .map(|message| ChatMessageRecord {
                    role: string_field(message, "role"),
                    content: string_field(message, "content"),
                })
                .collect();
            result.push(ChatSessionRecord {
                id: session_id,
                title: string_field(&session, "title"),
                messages,
            });
        }
        Ok(result)
    }

    pub async fn get_search_history(&self) -> Result<Vec<SearchHistoryRecord>, BackendError> {
        if !self.manager.is_db_ready() {
            return Ok(Vec::new());
        }

        let db = self.connect_search_database().await?;
        db.ensure_table("search_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("search_document")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        // Avoid selecting the legacy report/text blobs. Older records can
        // contain NUL characters that SurrealDB 2.1 cannot serialize safely.
        let sessions = db
            .query(
                "SELECT search_id, query, mode, status, warnings, model, source_outcomes, created_at, updated_at, knowledge_document, knowledge_ingested, synthesis, papers, total_results_found, pages_rendered FROM search_session ORDER BY created_at DESC LIMIT 50;",
            )
            .await
            .map_err(BackendError::from)?;
        let documents = db
            .query(
                "SELECT document_id, search_id, name, path, source_url, mime_type, size_bytes, status, extraction_method, ocr_operation_id, chars, chunks, processed_at, ingested_at, created_at FROM search_document ORDER BY created_at ASC;",
            )
            .await
            .map_err(BackendError::from)?;
        let ingested_knowledge_paths: HashSet<String> = db
            .query("SELECT path FROM kb_file WHERE source = 'meta_search_knowledge' AND status = 'committed' AND name NOT IN (SELECT VALUE name FROM knowledge_operation WHERE status != 'committed');")
            .await
            .map_err(BackendError::from)?
            .iter()
            .map(|record| string_field(record, "path"))
            .filter(|path| !path.is_empty())
            .collect();

        Ok(build_search_history(
            &sessions,
            &documents,
            &ingested_knowledge_paths,
        ))
    }

    pub async fn get_search_document_content(
        &self,
        search_id: &str,
        document_id: &str,
    ) -> Result<Value, BackendError> {
        let db = self.connect_search_database().await?;
        db.ensure_table("search_document")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("document_id".into(), Value::String(document_id.to_string()));
        db.query_with_vars(
            "SELECT * FROM search_document WHERE search_id = $search_id AND document_id = $document_id LIMIT 1;",
            variables,
        )
        .await
        .map_err(BackendError::from)?
        .into_iter()
        .next()
        .ok_or_else(|| BackendError::new("not_found", "Search document not found"))
    }

    pub async fn search_documents(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<DocumentSearchResult>, BackendError> {
        let document_store =
            self.manager.document_store.as_ref().ok_or_else(|| {
                BackendError::new("document_store_offline", "Document store offline")
            })?;
        Ok(document_store
            .search(query, limit as u64)
            .await
            .map_err(BackendError::from)?
            .into_iter()
            .map(|(text, score)| DocumentSearchResult { text, score })
            .collect())
    }

    pub async fn save_chat_session(
        &self,
        session_id: &str,
        title: &str,
        messages_json: &str,
    ) -> Result<(), BackendError> {
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        validate_session_payload_size(messages_json.len())?;

        let db = self.connect_database().await?;
        db.ensure_table("chat_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("chat_message")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("sid".into(), Value::String(session_id.to_string()));
        // Preserve the existing replace semantics and plain `session_id`
        // field; SurrealDB's reserved `id` would become a record reference.
        let _ = db
            .exec_with_vars(
                "DELETE chat_message WHERE session_id = $sid;",
                variables.clone(),
            )
            .await;
        let _ = db
            .exec_with_vars("DELETE chat_session WHERE session_id = $sid;", variables)
            .await;
        db.insert(
            "chat_session",
            serde_json::json!({
                "session_id": session_id,
                "title": title,
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }),
        )
        .await
        .map_err(BackendError::from)?;

        let messages: Vec<Value> = serde_json::from_str(messages_json).unwrap_or_default();
        for message in messages {
            db.insert(
                "chat_message",
                serde_json::json!({
                    "session_id": session_id,
                    "role": message["role"],
                    "content": message["content"],
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }),
            )
            .await
            .map_err(BackendError::from)?;
        }
        Ok(())
    }

    pub async fn delete_chat_session(&self, session_id: &str) -> Result<(), BackendError> {
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        let db = self.connect_database().await?;
        db.ensure_table("chat_session")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("chat_message")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("sid".into(), Value::String(session_id.to_string()));
        let _ = db
            .exec_with_vars(
                "DELETE chat_message WHERE session_id = $sid;",
                variables.clone(),
            )
            .await;
        let _ = db
            .exec_with_vars("DELETE chat_session WHERE session_id = $sid;", variables)
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_search_document_result(
        &self,
        search_id: &str,
        document_id: &str,
        method: &str,
        text: &str,
        chars: u32,
        chunks: u32,
        ocr_operation_id: Option<&str>,
    ) -> Result<(), BackendError> {
        validate_search_document_input(method, text.len())?;

        let db = self.connect_search_database().await?;
        db.ensure_table("search_document")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("document_id".into(), Value::String(document_id.to_string()));
        variables.insert("method".into(), Value::String(method.to_string()));
        variables.insert("text".into(), Value::String(text.to_string()));
        variables.insert("chars".into(), serde_json::json!(chars));
        variables.insert("chunks".into(), serde_json::json!(chunks));
        variables.insert(
            "ocr_operation_id".into(),
            serde_json::json!(ocr_operation_id),
        );
        variables.insert("now".into(), Value::String(chrono::Utc::now().to_rfc3339()));
        db.exec_with_vars(
            "UPDATE search_document SET status = 'processed', extraction_method = $method, ocr_operation_id = $ocr_operation_id, extracted_text = $text, chars = $chars, chunks = $chunks, processed_at = $now, updated_at = $now WHERE search_id = $search_id AND document_id = $document_id RETURN NONE;",
            variables,
        )
        .await
        .map_err(BackendError::from)
    }

    pub async fn mark_search_document_ingested(
        &self,
        search_id: &str,
        document_id: &str,
    ) -> Result<(), BackendError> {
        let db = self.connect_search_database().await?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("document_id".into(), Value::String(document_id.to_string()));
        variables.insert("now".into(), Value::String(chrono::Utc::now().to_rfc3339()));
        db.exec_with_vars(
            "UPDATE search_document SET status = 'ingested', ingested_at = $now, updated_at = $now WHERE search_id = $search_id AND document_id = $document_id RETURN NONE;",
            variables,
        )
        .await
        .map_err(BackendError::from)
    }

    pub async fn mark_search_knowledge_ingested(
        &self,
        search_id: &str,
    ) -> Result<(), BackendError> {
        let db = self.connect_search_database().await?;
        let mut variables = serde_json::Map::new();
        variables.insert("search_id".into(), Value::String(search_id.to_string()));
        variables.insert("now".into(), Value::String(chrono::Utc::now().to_rfc3339()));
        db.exec_with_vars(
            "UPDATE search_session SET knowledge_ingested = true, updated_at = $now WHERE search_id = $search_id RETURN NONE;",
            variables,
        )
        .await
        .map_err(BackendError::from)
    }
}
