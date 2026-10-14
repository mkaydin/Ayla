//! Backend code analysis operations.

use super::*;

impl BackendApi<'_> {
    pub async fn run_code_analysis(
        &self,
        mut request: CodeAnalysisRequest,
        events: &dyn EventSink,
        processes: &ProcessRegistry,
    ) -> Result<CodeAnalysisResult, BackendError> {
        let parent = request
            .parent_operation_id
            .as_deref()
            .map(crate::artifacts::load_code_analysis_run)
            .transpose()?;
        if let Some(parent) = &parent {
            if request.source_type == "snippet" {
                if request.name.is_none() {
                    request.name = Some(parent.source_label.clone());
                }
                if request.language.is_none() {
                    request.language = parent.language.clone();
                }
                if request
                    .code
                    .as_deref()
                    .map(str::trim)
                    .map_or(true, str::is_empty)
                    && parent.source_snapshot_stored
                {
                    request.code = Some(crate::artifacts::load_code_analysis_private_source(
                        &parent.operation_id,
                    )?);
                }
            }
        }
        if parent
            .as_ref()
            .is_some_and(|parent| parent.source_type != request.source_type)
        {
            return Err(BackendError::new(
                "code_analysis_parent_mismatch",
                "Re-analysis must use the same source type as its parent run",
            ));
        }
        if let Some(parent) = &parent {
            validate_code_analysis_parent(&request, parent)?;
        }
        let source = validate_code_analysis_source(&request)?;
        let mut options = ayla_code_analysis::GenerateOptions::default();
        options.allow_remote_git = false;
        options.allow_ssh_git = false;
        options.output_name = request.name.clone();
        options.source_scope = match request.scope.as_deref().unwrap_or("documents") {
            "documents" | "documents_only" => ayla_code_analysis::SourceScope::DocumentsOnly,
            "all" => ayla_code_analysis::SourceScope::All,
            _ => {
                return Err(BackendError::new(
                    "invalid_code_analysis_scope",
                    "Code-analysis scope must be 'documents' or 'all'",
                ))
            }
        };
        let semantic_mode = request
            .semantic_mode
            .as_deref()
            .unwrap_or("deterministic")
            .trim()
            .to_ascii_lowercase();
        match semantic_mode.as_str() {
            "deterministic" => options.semantic_analysis = false,
            "local" | "ollama" => {
                let model = self.manager.config.code_analysis_model.trim();
                let cloud_model = model.to_ascii_lowercase().contains(":cloud");
                if model.is_empty() {
                    return Err(BackendError::new(
                        "local_semantic_model_invalid",
                        "Semantic enrichment requires AYLA_CODE_ANALYSIS_MODEL",
                    ));
                }
                if cloud_model && semantic_mode == "local" {
                    return Err(BackendError::new(
                        "local_semantic_model_invalid",
                        "Local semantic enrichment cannot use a cloud model; choose configured Ollama mode",
                    ));
                }
                if cloud_model && !self.manager.config.code_analysis_allow_cloud {
                    return Err(BackendError::new(
                        "code_semantic_cloud_disabled",
                        "Cloud code analysis requires AYLA_CODE_ANALYSIS_ALLOW_CLOUD=true",
                    ));
                }
                options.ollama_base_url =
                    self.manager.config.ollama_url.parse().map_err(|error| {
                        BackendError::new(
                            "local_semantic_url_invalid",
                            format!("The configured Ollama URL is invalid: {error}"),
                        )
                    })?;
                let local_host = matches!(
                    options.ollama_base_url.host_str(),
                    Some("localhost" | "127.0.0.1" | "::1")
                );
                let local_transport = matches!(options.ollama_base_url.scheme(), "http" | "https")
                    && options.ollama_base_url.username().is_empty()
                    && options.ollama_base_url.password().is_none();
                if !local_host || !local_transport {
                    return Err(BackendError::new(
                        "local_semantic_remote_blocked",
                        "Semantic enrichment only permits an unauthenticated HTTP(S) loopback Ollama endpoint",
                    ));
                }
                options.model = model.to_string();
                options.semantic_analysis = true;
            }
            _ => {
                return Err(BackendError::new(
                    "invalid_code_semantic_mode",
                    "Semantic mode must be 'deterministic', 'local', or 'ollama'",
                ))
            }
        }

        let pid = uuid::Uuid::new_v4().to_string();
        let cancel_rx = processes.register_task(pid.clone()).await;
        let _process_cleanup = processes.cleanup_on_drop(pid.clone());
        let _ = events.emit("code-analysis-started", serde_json::json!({ "pid": pid }));
        let mut cancel_rx = cancel_rx;
        let documents = if semantic_mode != "deterministic" {
            let _ = events.emit(
                "code-analysis-progress",
                serde_json::json!({
                    "pid": pid,
                    "stage": "semantic",
                    "current": 0,
                    "total": 1,
                    "message": "Analyzing structure with the configured Ollama semantic model"
                }),
            );
            let generator = ayla_code_analysis::Generator::with_llm(
                ayla_code_analysis::OllamaClient::default(),
            );
            let mut worker = tokio::spawn(async move {
                ayla_code_analysis::CodeDocumentationGenerator::generate(
                    &generator, source, options,
                )
                .await
            });
            let _worker_guard = CodeAnalysisAsyncWorkerGuard(worker.abort_handle());
            tokio::select! {
                result = &mut worker => result.map_err(|error| BackendError::new(
                    "code_analysis_task",
                    format!("Code-analysis task panicked: {error}"),
                ))?.map_err(code_analysis_error)?,
                _ = &mut cancel_rx => {
                    worker.abort();
                    let _ = worker.await;
                    processes.finish(&pid).await;
                    return Err(BackendError::new(
                        "code_analysis_cancelled",
                        "Code analysis cancelled",
                    ));
                }
            }
        } else {
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
            let control = ayla_code_analysis::AnalysisControl::with_progress(move |update| {
                // The API emits the terminal event synchronously before the
                // response, avoiding a worker/channel race and duplicates.
                if update.stage != "complete" {
                    let _ = progress_tx.send(update);
                }
            });
            // Renderer/request cancellation may drop this future before its select
            // branch runs. The guard still signals the blocking parser to stop.
            let _worker_guard = CodeAnalysisWorkerGuard(control.clone());
            let worker_control = control.clone();
            let mut worker = tokio::task::spawn_blocking(move || {
                ayla_code_analysis::generate_deterministic(source, options, &worker_control)
            });
            let mut progress_open = true;
            loop {
                tokio::select! {
                    result = &mut worker => {
                        break result.map_err(|error| BackendError::new(
                            "code_analysis_task",
                            format!("Code-analysis task panicked: {error}"),
                        ))?.map_err(code_analysis_error)?;
                    }
                    _ = &mut cancel_rx => {
                        control.cancel();
                        let _ = worker.await;
                        processes.finish(&pid).await;
                        return Err(BackendError::new(
                            "code_analysis_cancelled",
                            "Code analysis cancelled",
                        ));
                    }
                    progress = progress_rx.recv(), if progress_open => {
                        match progress {
                            Some(progress) => {
                                let mut payload = serde_json::to_value(progress).unwrap_or_default();
                                payload["pid"] = Value::String(pid.clone());
                                let _ = events.emit("code-analysis-progress", payload);
                            }
                            None => progress_open = false,
                        }
                    }
                }
            }
        };
        processes.finish(&pid).await;
        let _ = events.emit(
            "code-analysis-progress",
            serde_json::json!({
                "pid": pid,
                "stage": "complete",
                "current": 1,
                "total": 1,
                "message": "Code analysis complete"
            }),
        );

        let timestamp = chrono::Utc::now().to_rfc3339();
        let batch = GraphImportBatch::from_code_analysis(
            format!("code-analysis:{pid}"),
            &documents,
            &timestamp,
        );
        let validation = batch.validate();
        if !validation.is_valid() {
            return Err(BackendError::new(
                "code_analysis_graph_invalid",
                format!(
                    "Generated graph preview failed validation: {}",
                    validation.errors.join("; ")
                ),
            ));
        }
        let mut artifacts = crate::artifacts::persist_code_analysis_artifacts(&pid, &documents)?;
        let diff = if let Some(parent) = &parent {
            let previous = crate::artifacts::load_code_analysis_result(&parent.operation_id)?;
            let (summary, artifact) = crate::artifacts::persist_code_analysis_diff(
                &pid,
                &parent.operation_id,
                &documents,
                &previous,
            )?;
            artifacts.diff = Some(artifact);
            Some(summary)
        } else {
            None
        };
        let source_snapshot_stored = if request.source_type == "snippet" {
            let source = request.code.as_deref().ok_or_else(|| {
                BackendError::new(
                    "private_source_missing",
                    "The analyzed snippet source is unavailable for private persistence",
                )
            })?;
            crate::artifacts::persist_code_analysis_private_source(&pid, source)?;
            true
        } else {
            false
        };
        let source_label = if request.source_type == "directory" {
            request
                .directory_path
                .as_deref()
                .and_then(|path| std::path::Path::new(path).file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("Local directory")
                .to_string()
        } else {
            request.name.clone().unwrap_or_else(|| "snippet.rs".into())
        };
        let run = crate::artifacts::CodeAnalysisRunRecord {
            schema_version: 2,
            operation_id: pid.clone(),
            parent_operation_id: parent.as_ref().map(|run| run.operation_id.clone()),
            created_at: timestamp.clone(),
            updated_at: timestamp.clone(),
            status: "preview".into(),
            source_type: request.source_type.clone(),
            source_label,
            directory_path: request.directory_path.clone(),
            language: request.language.clone(),
            scope: request.scope.clone().unwrap_or_else(|| "documents".into()),
            semantic_mode: semantic_mode.clone(),
            project_name: documents.project_name.clone(),
            repository_uid: documents.repository_uid.clone(),
            entity_count: documents.entities.len(),
            relation_count: documents.relations.len(),
            diagnostics: documents.diagnostics.clone(),
            artifacts: artifacts.clone(),
            diff: diff.clone(),
            graph_approved: false,
            vectors_indexed: false,
            source_snapshot_stored,
        };
        crate::artifacts::persist_code_analysis_run(&run)?;
        Ok(CodeAnalysisResult {
            pid,
            project_name: documents.project_name,
            repository_uid: documents.repository_uid,
            entity_count: documents.entities.len(),
            relation_count: documents.relations.len(),
            diagnostics: documents.diagnostics,
            validation,
            artifacts,
            parent_operation_id: run.parent_operation_id,
            created_at: timestamp,
            semantic_mode,
            diff,
            graph_approved: false,
            vectors_indexed: false,
            source_snapshot_stored,
        })
    }

    pub fn get_code_analysis_runs(
        &self,
    ) -> Result<Vec<crate::artifacts::CodeAnalysisRunRecord>, BackendError> {
        crate::artifacts::list_code_analysis_runs()
    }

    pub fn get_code_analysis_run(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisResult, BackendError> {
        let run = crate::artifacts::load_code_analysis_run(operation_id)?;
        let documents = crate::artifacts::load_code_analysis_result(operation_id)?;
        let batch = GraphImportBatch::from_code_analysis(
            format!("code-analysis:{operation_id}"),
            &documents,
            &run.updated_at,
        );
        let validation = batch.validate();
        Ok(CodeAnalysisResult {
            pid: run.operation_id,
            project_name: run.project_name,
            repository_uid: run.repository_uid,
            entity_count: run.entity_count,
            relation_count: run.relation_count,
            diagnostics: run.diagnostics,
            validation,
            artifacts: run.artifacts,
            parent_operation_id: run.parent_operation_id,
            created_at: run.created_at,
            semantic_mode: run.semantic_mode,
            diff: run.diff,
            graph_approved: run.graph_approved,
            vectors_indexed: run.vectors_indexed,
            source_snapshot_stored: run.source_snapshot_stored,
        })
    }

    pub fn get_artifact_content(
        &self,
        artifact_id: &str,
    ) -> Result<crate::ArtifactContent, BackendError> {
        crate::artifacts::read_artifact_content(artifact_id)
    }

    pub async fn approve_code_analysis(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisApproval, BackendError> {
        let run = crate::artifacts::load_code_analysis_run(operation_id)?;
        if run.graph_approved {
            return Err(BackendError::new(
                "code_analysis_already_approved",
                "This code-analysis run is already approved",
            ));
        }
        let approved_parent = run
            .parent_operation_id
            .as_deref()
            .and_then(|parent_id| crate::artifacts::load_code_analysis_run(parent_id).ok())
            .filter(|parent| parent.graph_approved);
        let documents = crate::artifacts::load_code_analysis_result(operation_id)?;
        let source_id = format!("code-analysis:{operation_id}");
        let batch = GraphImportBatch::from_code_analysis(
            source_id.clone(),
            &documents,
            &chrono::Utc::now().to_rfc3339(),
        );
        let validation = batch.validate();
        if !validation.is_valid() {
            return Err(BackendError::new(
                "code_analysis_graph_invalid",
                format!("Graph-v2 import rejected: {}", validation.errors.join("; ")),
            ));
        }
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let vector_store = self.manager.code_analysis_vectors.as_ref().ok_or_else(|| {
            BackendError::new(
                "code_analysis_vectors_offline",
                "Code-analysis vector storage is unavailable while Qdrant is offline",
            )
        })?;
        let vector_receipt = match vector_store.index_run(operation_id, &documents).await {
            Ok(receipt) => receipt,
            Err(error) => {
                return Err(BackendError::new(
                    "code_analysis_vector_index",
                    format!("Could not index code-analysis vectors: {error}"),
                ));
            }
        };
        let graph_result = if let Some(parent) = &approved_parent {
            store
                .lock()
                .await
                .replace_source(&format!("code-analysis:{}", parent.operation_id), batch)
                .await
        } else {
            store.lock().await.import(batch).await
        };
        let receipt = match graph_result {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = vector_store.delete_run(operation_id).await;
                return Err(BackendError::new("graph_v2_import", error));
            }
        };
        if let Err(error) = crate::artifacts::update_code_analysis_run(operation_id, |run| {
            run.status = "approved".into();
            run.graph_approved = true;
            run.vectors_indexed = true;
        }) {
            let _ = vector_store.delete_run(operation_id).await;
            let _ = store.lock().await.rollback(&receipt.backup_id).await;
            return Err(error);
        }
        if let Some(parent) = &approved_parent {
            if let Err(error) =
                crate::artifacts::update_code_analysis_run(&parent.operation_id, |run| {
                    run.status = "superseded".into();
                    run.graph_approved = false;
                })
            {
                let _ = vector_store.delete_run(operation_id).await;
                let _ = store.lock().await.rollback(&receipt.backup_id).await;
                let _ = crate::artifacts::update_code_analysis_run(operation_id, |run| {
                    run.status = "preview".into();
                    run.graph_approved = false;
                    run.vectors_indexed = false;
                });
                return Err(error);
            }
        }
        Ok(CodeAnalysisApproval {
            operation_id: operation_id.into(),
            source_id,
            receipt,
            entity_vectors: vector_receipt.entity_vectors,
            document_vectors: vector_receipt.document_vectors,
        })
    }

    pub async fn remove_code_analysis(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisRemoval, BackendError> {
        // Requiring the corresponding artifact prevents this general-purpose
        // endpoint from being used to remove arbitrary provenance IDs.
        let _ = crate::artifacts::load_code_analysis_result(operation_id)?;
        let source_id = format!("code-analysis:{operation_id}");
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
        crate::artifacts::update_code_analysis_run(operation_id, |run| {
            run.status = "preview".into();
            run.graph_approved = false;
        })?;
        Ok(CodeAnalysisRemoval {
            operation_id: operation_id.into(),
            source_id,
            receipt,
        })
    }

    pub async fn delete_code_analysis_run(
        &self,
        operation_id: &str,
    ) -> Result<CodeAnalysisRunDeletion, BackendError> {
        let run = crate::artifacts::load_code_analysis_run(operation_id)?;
        let graph_store = if run.graph_approved {
            Some(self.manager.graph_v2.as_ref().ok_or_else(|| {
                BackendError::new(
                    "graph_v2_offline",
                    "Cannot delete an approved run while graph-v2 storage is offline",
                )
            })?)
        } else {
            self.manager.graph_v2.as_ref()
        };
        let vector_store = if run.vectors_indexed {
            Some(self.manager.code_analysis_vectors.as_ref().ok_or_else(|| {
                BackendError::new(
                    "code_analysis_vectors_offline",
                    "Cannot delete an indexed run while Qdrant is offline",
                )
            })?)
        } else {
            self.manager.code_analysis_vectors.as_ref()
        };

        crate::artifacts::update_code_analysis_run(operation_id, |run| {
            run.status = "deleting".into();
        })?;

        let mut graph_removed = false;
        if let Some(store) = graph_store {
            store
                .lock()
                .await
                .remove_source(&format!("code-analysis:{operation_id}"))
                .await
                .map_err(|error| BackendError::new("graph_v2_remove", error))?;
            graph_removed = true;
            crate::artifacts::update_code_analysis_run(operation_id, |run| {
                run.graph_approved = false;
            })?;
        }

        let mut vectors_removed = false;
        if let Some(store) = vector_store {
            if let Err(error) = store.delete_run(operation_id).await {
                let _ = crate::artifacts::update_code_analysis_run(operation_id, |run| {
                    run.status = "delete_failed".into();
                });
                return Err(BackendError::new(
                    "code_analysis_vector_delete",
                    format!("Could not delete code-analysis vectors: {error}"),
                ));
            }
            vectors_removed = true;
            crate::artifacts::update_code_analysis_run(operation_id, |run| {
                run.vectors_indexed = false;
            })?;
        }

        if let Err(error) = crate::artifacts::delete_code_analysis_artifacts(operation_id) {
            let _ = crate::artifacts::update_code_analysis_run(operation_id, |run| {
                run.status = "delete_failed".into();
            });
            return Err(error);
        }
        Ok(CodeAnalysisRunDeletion {
            operation_id: operation_id.into(),
            graph_removed,
            vectors_removed,
            artifacts_removed: true,
        })
    }
}
