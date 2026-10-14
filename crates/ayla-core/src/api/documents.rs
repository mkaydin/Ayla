//! Backend documents operations.

use super::*;

impl BackendApi<'_> {
    pub async fn commit_document(
        &self,
        name: String,
        path: String,
        chars: u32,
        source_type: String,
        text: String,
        events: &dyn EventSink,
    ) -> Result<String, BackendError> {
        let _knowledge_writer = KNOWLEDGE_WRITER.lock().await;
        validate_file_payload_size(text.len())?;
        let db = self.connect_search_database().await?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        let identity = serde_json::json!({"name": name})
            .as_object()
            .unwrap()
            .clone();
        let existing = db
            .query_with_vars(
                "SELECT path FROM kb_file WHERE name = $name LIMIT 2;",
                identity,
            )
            .await
            .map_err(BackendError::from)?;
        if existing.iter().any(|record| {
            record
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|old| old != path)
        }) {
            return Err(BackendError::new("duplicate_document_name", "A different document already uses this name. Rename it before importing; the existing source was preserved."));
        }
        db.ensure_table("knowledge_operation")
            .await
            .map_err(BackendError::from)?;
        let vars = serde_json::json!({"name": name, "path": path, "text": text, "source_type": source_type, "chars": chars}).as_object().unwrap().clone();
        db.exec_with_vars("BEGIN TRANSACTION; DELETE knowledge_operation WHERE name = $name; CREATE knowledge_operation CONTENT { name: $name, path: $path, text: $text, source_type: $source_type, chars: $chars, status: 'approving' }; COMMIT TRANSACTION;", vars.clone()).await.map_err(BackendError::from)?;
        let committed_text = text.clone();
        let result = async {
            self.save_file_record(name.clone(), path, chars, source_type.clone(), text.clone())
                .await?;
            if source_type == "ocr_knowledge" {
                let operation_id = path_ocr_operation(&vars["path"].as_str().unwrap_or_default())
                    .ok_or_else(|| BackendError::new("invalid_ocr_source", "Missing OCR operation identity"))?;
                self.approve_ocr_graph(operation_id).await?;
                db.ensure_table("text_store").await.map_err(BackendError::from)?;
                db.exec_with_vars("BEGIN TRANSACTION; DELETE text_store WHERE key = $name; CREATE text_store CONTENT { key: $name, value: $text }; COMMIT TRANSACTION;", vars.clone()).await.map_err(BackendError::from)?;
                Ok(format!("Approved OCR evidence, graph, and vectors from {name}"))
            } else {
                {
                    let mut graph = self.manager.graph.lock().await;
                    graph.remove_by_source(&name);
                    graph.sync().await.map_err(BackendError::from)?;
                }
                self.ingest_text(text, name.clone(), source_type, events).await
            }
        }
        .await;
        match result {
            Ok(message) => {
                db.exec_with_vars("UPDATE knowledge_operation SET status = 'committed', text = NONE WHERE name = $name RETURN NONE; UPDATE kb_file SET status = 'committed' WHERE name = $name RETURN NONE;", vars).await.map_err(BackendError::from)?;
                self.manager
                    .store_document_text(name.clone(), committed_text)
                    .await;
                let _ = events.emit(
                    "knowledge-changed",
                    serde_json::json!({"source": name, "action": "commit"}),
                );
                Ok(message)
            }
            Err(error) => {
                self.manager.remove_document_text(&name).await;
                let _ = db.exec_with_vars("UPDATE knowledge_operation SET status = 'failed' WHERE name = $name RETURN NONE; UPDATE kb_file SET status = 'failed' WHERE name = $name RETURN NONE;", vars).await;
                Err(error)
            }
        }
    }

    pub async fn save_file_record(
        &self,
        name: String,
        path: String,
        chars: u32,
        source_type: String,
        text: String,
    ) -> Result<(), BackendError> {
        log::info!(
            "File record: saving '{}' ({} chars, path={})",
            name,
            chars,
            crate::memory::safe_truncate(&path, 40)
        );
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        validate_file_payload_size(text.len())?;

        let db = self.connect_database().await?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;

        let vars = serde_json::json!({"name": name})
            .as_object()
            .unwrap()
            .clone();
        let existing = db
            .query_with_vars("SELECT path FROM kb_file WHERE name = $name LIMIT 2;", vars)
            .await
            .map_err(BackendError::from)?;
        if existing.iter().any(|record| {
            record
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|old| old != path)
        }) {
            return Err(BackendError::new("duplicate_document_name", "A different document already uses this name. Rename the incoming file or remove the existing source before importing it."));
        }
        let source_id = format!(
            "document:{:x}",
            sha2::Sha256::digest(format!("{name}\0{path}").as_bytes())
        );
        // Approval is the retrieval commit point. Discarded quick-extraction
        // previews must never leave orphan vectors behind.
        if let Some(document_store) = &self.manager.document_store {
            let chunks = crate::tools::quick_extract::chunk_text(&text, 1500);
            document_store
                .delete_by_filename(&name)
                .await
                .map_err(BackendError::from)?;
            document_store
                .store_chunks(&name, &chunks)
                .await
                .map_err(BackendError::from)?;
        }

        let mut variables = serde_json::Map::new();
        variables.insert("name".into(), Value::String(name.clone()));
        db.exec_with_vars("DELETE kb_file WHERE name = $name;", variables)
            .await
            .map_err(BackendError::from)?;
        let record = serde_json::json!({
            "name": name,
            "source_id": source_id,
            "status": "committed",
            "path": path,
            "chars": chars,
            "source": source_type,
            "date": chrono::Utc::now().to_rfc3339(),
            "text": text,
        });
        if let Err(error) = db.insert("kb_file", record).await {
            if let Some(document_store) = &self.manager.document_store {
                let _ = document_store.delete_by_filename(&name).await;
            }
            return Err(BackendError::from(error));
        }
        log::info!("File record: saved successfully");
        Ok(())
    }

    pub async fn delete_file_record(
        &self,
        name: &str,
        delete_nodes: bool,
    ) -> Result<String, BackendError> {
        let _knowledge_writer = KNOWLEDGE_WRITER.lock().await;
        if !self.manager.is_db_ready() {
            return Err(BackendError::new("db_offline", "DB offline"));
        }
        let db = self.connect_database().await?;
        db.ensure_table("kb_file")
            .await
            .map_err(BackendError::from)?;
        db.ensure_table("text_store")
            .await
            .map_err(BackendError::from)?;
        let mut variables = serde_json::Map::new();
        variables.insert("name".into(), Value::String(name.to_string()));
        let records = db
            .query_with_vars(
                "SELECT path FROM kb_file WHERE name = $name;",
                variables.clone(),
            )
            .await
            .map_err(BackendError::from)?;
        self.manager.remove_document_text(name).await;
        db.exec_with_vars(
            "UPDATE kb_file SET status = 'deleting' WHERE name = $name RETURN NONE;",
            variables.clone(),
        )
        .await
        .map_err(BackendError::from)?;
        // Do not acknowledge deletion while vectors survive. The durable tombstone
        // keeps the source out of retrieval and makes this operation safe to retry.
        if let Some(document_store) = &self.manager.document_store {
            document_store
                .delete_by_filename(name)
                .await
                .map_err(|e| BackendError::new("document_delete_incomplete", e))?;
        }
        let mut removed_nodes = 0;
        if delete_nodes {
            let mut graph = self.manager.graph.lock().await;
            removed_nodes = graph.remove_by_source(name);
            graph.sync().await.map_err(BackendError::from)?;
            drop(graph);
            for record in records {
                if let Some(operation) = record
                    .get("path")
                    .and_then(Value::as_str)
                    .and_then(path_ocr_operation)
                {
                    if let Some(store) = &self.manager.graph_v2 {
                        store
                            .lock()
                            .await
                            .remove_source(&format!("ocr:{operation}"))
                            .await
                            .map_err(BackendError::from)?;
                    }
                }
            }
        }
        db.exec_with_vars("BEGIN TRANSACTION; DELETE knowledge_operation WHERE name = $name; CREATE knowledge_operation CONTENT { name: $name, status: 'deleted' }; DELETE text_store WHERE key = $name; DELETE kb_file WHERE name = $name; COMMIT TRANSACTION;", variables).await.map_err(BackendError::from)?;
        self.manager.remove_document_text(name).await;
        log::info!(
            "File record: deleted '{}' ({} graph nodes removed)",
            name,
            removed_nodes
        );
        Ok(format!(
            "Deleted '{}' ({} graph nodes removed)",
            name, removed_nodes
        ))
    }

    pub async fn delete_file_nodes(&self, name: &str) -> Result<String, BackendError> {
        let mut graph = self.manager.graph.lock().await;
        let removed_nodes = graph.remove_by_source(name);
        graph.sync().await.map_err(BackendError::from)?;
        Ok(format!("Removed {} nodes from '{}'", removed_nodes, name))
    }
}
