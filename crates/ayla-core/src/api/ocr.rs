//! Backend ocr operations.

use super::*;

impl BackendApi<'_> {
    pub async fn run_ocr(
        &self,
        file_path: &str,
        knowledge_mode: Option<&str>,
        resources: &ResourceLocator,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<OcrResult, BackendError> {
        let validated = validate_source_file(file_path)?;
        let file_path = validated.to_string_lossy().to_string();
        let pid = uuid::Uuid::new_v4().to_string();
        log::info!("OCR [{}]: starting for {}", pid, file_path);

        let configured_mode = knowledge_mode.unwrap_or(&self.manager.config.ocr_knowledge_mode);
        let knowledge_mode =
            ayla_ocr::OcrKnowledgeMode::parse(configured_mode).ok_or_else(|| {
                BackendError::new(
                    "invalid_ocr_knowledge_mode",
                    "OCR knowledge mode must be disabled, deterministic, or semantic",
                )
            })?;
        let revision = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(client) => match client
                .get(format!("{}/api/tags", self.manager.config.ollama_url))
                .send()
                .await
            {
                Ok(response) => response.json::<Value>().await.ok().and_then(|value| {
                    value
                        .get("models")
                        .and_then(Value::as_array)
                        .and_then(|models| {
                            models.iter().find(|model| {
                                model.get("name").and_then(Value::as_str)
                                    == Some(self.manager.config.ocr_model.as_str())
                            })
                        })
                        .and_then(|model| model.get("digest").and_then(Value::as_str))
                        .map(str::to_string)
                }),
                Err(_) => None,
            },
            Err(_) => None,
        };
        let options = ayla_ocr::OcrOptions {
            model: self.manager.config.ocr_model.clone(),
            model_revision: revision.clone(),
            pdfium_library: Some(
                resources
                    .resolve_resource("lib/libpdfium.so", "target/native/pdfium/lib/libpdfium.so"),
            ),
            ollama_url: self.manager.config.ollama_url.clone(),
            parallel_pages: self.manager.config.ocr_parallel,
            num_ctx: self.manager.config.ocr_num_ctx as u32,
            request_timeout_secs: self.manager.config.ollama_long_timeout_secs as u64,
            resume_cache: self.manager.config.ocr_resume_cache && revision.is_some(),
            cache_dir: Some(self.manager.config.ocr_cache_dir.clone()),
            knowledge_mode,
            assembly_model: Some(self.manager.config.ocr_assembly_model.clone()),
            assembly_allow_cloud: self.manager.config.ocr_assembly_allow_cloud,
            ..Default::default()
        };
        let cancel_rx = processes.register_task(pid.clone()).await;
        let _process_cleanup = processes.cleanup_on_drop(pid.clone());
        let _ = events.emit("ocr-started", serde_json::json!({ "pid": pid }));
        let progress_pid = pid.clone();
        let progress = |update: ayla_ocr::OcrProgress| {
            let mut payload = serde_json::to_value(update).unwrap_or_default();
            payload["pid"] = Value::String(progress_pid.clone());
            let _ = events.emit("ocr-progress", payload);
        };
        let document = tokio::select! {
            result = ayla_ocr::extract_pdf(&file_path, options, Some(&progress)) => {
                processes.finish(&pid).await;
                result.map_err(|error| BackendError::new("ocr_failed", error.to_string()))?
            }
            _ = cancel_rx => {
                processes.finish(&pid).await;
                return Err(BackendError::new("ocr_cancelled", "OCR cancelled"));
            }
        };
        let knowledge_batch = GraphImportBatch::from_ocr_knowledge(
            format!("ocr:{pid}"),
            &document.knowledge,
            &chrono::Utc::now().to_rfc3339(),
        );
        let knowledge_validation = knowledge_batch.validate();
        if !knowledge_validation.is_valid() {
            return Err(BackendError::new(
                "ocr_knowledge_invalid",
                format!(
                    "OCR knowledge preview failed validation: {}",
                    knowledge_validation.errors.join("; ")
                ),
            ));
        }
        let artifacts = crate::artifacts::persist_ocr_artifacts(&pid, &document)?;
        log::info!("OCR [{}]: done — {} chars", pid, document.text.len());
        Ok(OcrResult {
            chars: document.text.len(),
            pages: document.pages_processed.to_string(),
            pid,
            source_name: document.source_name,
            pages_processed: document.pages_processed,
            pages_skipped: document.pages_skipped,
            total_pages: document.total_pages,
            page_results: document.page_results,
            models: document.models,
            warnings: document.warnings,
            knowledge_mode: document.knowledge_mode,
            performance: document.performance,
            cache_key: document.cache_key,
            markdown_artifact: artifacts.markdown,
            layout_artifact: artifacts.layout,
            knowledge_base_artifact: artifacts.knowledge_base,
            structured_artifact: artifacts.structured,
            knowledge_entity_count: document.knowledge.entities.len(),
            knowledge_relation_count: document.knowledge.relations.len(),
            knowledge_validation,
            text: document.text,
        })
    }

    pub async fn ingest_ocr_knowledge(
        &self,
        operation_id: &str,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<OcrKnowledgeIngestion, BackendError> {
        let cancel_rx = processes.register_task(operation_id.into()).await;
        let _process_cleanup = processes.cleanup_on_drop(operation_id);
        let ingestion = async {
            let knowledge = crate::artifacts::load_ocr_knowledge_result(operation_id)?;
            let markdown = knowledge.knowledge_base_markdown;
            validate_ingest_payload_size(markdown.len())?;
            let chars = markdown.chars().count();
            let message = self
                .commit_document(
                    knowledge.source_name.clone(),
                    format!("ocr://{operation_id}/{}", knowledge.source_name),
                    chars.min(u32::MAX as usize) as u32,
                    "ocr_knowledge".into(),
                    markdown,
                    events,
                )
                .await?;
            Ok(OcrKnowledgeIngestion {
                operation_id: operation_id.into(),
                source_name: knowledge.source_name,
                chars,
                message,
            })
        };
        let result = tokio::select! {
            result = ingestion => result,
            _ = cancel_rx => Err(BackendError::new(
                "ocr_knowledge_cancelled",
                "OCR knowledge ingestion cancelled",
            )),
        };
        processes.finish(operation_id).await;
        result
    }

    pub async fn approve_ocr_graph(
        &self,
        operation_id: &str,
    ) -> Result<OcrGraphMutation, BackendError> {
        let knowledge = crate::artifacts::load_ocr_knowledge_result(operation_id)?;
        let source_id = format!("ocr:{operation_id}");
        let batch = GraphImportBatch::from_ocr_knowledge(
            source_id.clone(),
            &knowledge,
            &chrono::Utc::now().to_rfc3339(),
        );
        let validation = batch.validate();
        if !validation.is_valid() {
            return Err(BackendError::new(
                "ocr_knowledge_invalid",
                format!(
                    "OCR graph-v2 import rejected: {}",
                    validation.errors.join("; ")
                ),
            ));
        }
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let receipt = store
            .lock()
            .await
            .import(batch)
            .await
            .map_err(|error| BackendError::new("graph_v2_import", error))?;
        Ok(OcrGraphMutation {
            operation_id: operation_id.into(),
            source_id,
            receipt,
        })
    }

    pub async fn remove_ocr_graph(
        &self,
        operation_id: &str,
    ) -> Result<OcrGraphMutation, BackendError> {
        let _ = crate::artifacts::load_ocr_knowledge_result(operation_id)?;
        let source_id = format!("ocr:{operation_id}");
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let receipt = store
            .lock()
            .await
            .remove_source(&source_id)
            .await
            .map_err(|error| BackendError::new("graph_v2_remove", error))?;
        Ok(OcrGraphMutation {
            operation_id: operation_id.into(),
            source_id,
            receipt,
        })
    }
}
