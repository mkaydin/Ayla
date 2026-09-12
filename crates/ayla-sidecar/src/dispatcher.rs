use ayla_core::{BackendError, BackendState, EventSink, StreamSink};
use rmpv::Value;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub async fn dispatch(
    state: &BackendState,
    method: &str,
    params: Value,
    events: &dyn EventSink,
    stream: &dyn StreamSink,
) -> Result<Value, BackendError> {
    let api = state.api();
    match method {
        // Group A — reads
        "get_runtime_capabilities" => encode(state.runtime_capabilities()),
        "get_stt_status" => encode(state.stt().status()),
        "set_system_stt_enabled" => {
            let params: SetSystemSttEnabledParams = decode(params)?;
            encode(state.stt().set_system_enabled(params.enabled))
        }
        "check_db_ready" => encode(api.check_db_ready()),
        "get_kb_data" => encode(api.get_kb_data().await),
        "get_graph_v2_data" => encode(api.get_graph_v2_data().await?),
        "get_graph_v2_page" => {
            let params: GraphV2PageParams = decode(params)?;
            encode(
                api.get_graph_v2_page(&params.kind, params.offset, params.limit)
                    .await?,
            )
        }
        "get_files" => encode(api.get_files().await?),
        "get_document_text" => {
            let params: NameParams = decode(params)?;
            encode(api.get_document_text(&params.name).await)
        }
        "load_chat_sessions" => encode(api.load_chat_sessions().await?),
        "get_search_history" => encode(api.get_search_history().await?),
        "get_search_document_content" => {
            let params: SearchDocumentParams = decode(params)?;
            encode(
                api.get_search_document_content(&params.search_id, &params.document_id)
                    .await?,
            )
        }
        "get_artifact_content" => {
            let params: ArtifactIdParams = decode(params)?;
            encode(api.get_artifact_content(&params.artifact_id)?)
        }
        "get_code_analysis_runs" => encode(api.get_code_analysis_runs()?),
        "get_code_analysis_run" => {
            let params: OperationIdParams = decode(params)?;
            encode(api.get_code_analysis_run(&params.operation_id)?)
        }
        "search_documents" => {
            let params: SearchDocumentsParams = decode(params)?;
            encode(api.search_documents(&params.query, params.limit).await?)
        }

        // Group C — memory and graph work
        "enrich_prompt" => {
            let params: EnrichPromptParams = decode(params)?;
            encode(
                api.enrich_prompt(&params.user_query, &params.base_prompt)
                    .await?,
            )
        }
        "process_turn" => {
            let params: ProcessTurnParams = decode(params)?;
            api.process_turn(&params.user_msg, &params.assistant_msg)
                .await?;
            Ok(Value::Nil)
        }
        "process_message" => {
            let params: ProcessMessageParams = decode(params)?;
            encode(
                api.process_message(&params.query, &params.base_prompt)
                    .await?,
            )
        }
        "ingest_text" => {
            let params: IngestTextParams = decode(params)?;
            encode(
                api.ingest_text(params.text, params.source, params.source_type, events)
                    .await?,
            )
        }
        "run_consolidation" => encode(api.run_consolidation().await?),
        "analyze_and_merge_kb" => encode(api.analyze_and_merge_kb(events).await?),
        "approve_code_analysis" => {
            let params: OperationIdParams = decode(params)?;
            encode(api.approve_code_analysis(&params.operation_id).await?)
        }
        "ingest_ocr_knowledge" => {
            let params: OperationIdParams = decode(params)?;
            encode(
                api.ingest_ocr_knowledge(&params.operation_id, events, state.processes())
                    .await?,
            )
        }
        "approve_ocr_graph" => {
            let params: OperationIdParams = decode(params)?;
            encode(api.approve_ocr_graph(&params.operation_id).await?)
        }
        "remove_ocr_graph" => {
            let params: OperationIdParams = decode(params)?;
            encode(api.remove_ocr_graph(&params.operation_id).await?)
        }
        "rollback_graph_v2" => {
            let params: BackupIdParams = decode(params)?;
            encode(api.rollback_graph_v2(&params.backup_id).await?)
        }
        "migrate_legacy_graph_v2" => encode(api.migrate_legacy_graph_v2().await?),
        "prune_graph_v2_backups" => {
            let params: GraphV2PruneParams = decode(params)?;
            encode(api.prune_graph_v2_backups(params.retention).await?)
        }
        "remove_code_analysis" => {
            let params: OperationIdParams = decode(params)?;
            encode(api.remove_code_analysis(&params.operation_id).await?)
        }
        "delete_code_analysis_run" => {
            let params: OperationIdParams = decode(params)?;
            encode(api.delete_code_analysis_run(&params.operation_id).await?)
        }

        // Group B — mutations
        "save_chat_session" => {
            let params: SaveChatSessionParams = decode(params)?;
            api.save_chat_session(&params.session_id, &params.title, &params.messages_json)
                .await?;
            Ok(Value::Nil)
        }
        "delete_chat_session" => {
            let params: SessionIdParams = decode(params)?;
            api.delete_chat_session(&params.session_id).await?;
            Ok(Value::Nil)
        }
        "commit_document" => {
            let p: SaveFileRecordParams = decode(params)?;
            encode(
                api.commit_document(p.name, p.path, p.chars, p.source_type, p.text, events)
                    .await?,
            )
        }
        "save_file_record" => {
            let params: SaveFileRecordParams = decode(params)?;
            api.save_file_record(
                params.name,
                params.path,
                params.chars,
                params.source_type,
                params.text,
            )
            .await?;
            Ok(Value::Nil)
        }
        "delete_file_record" => {
            let params: DeleteFileRecordParams = decode(params)?;
            encode(
                api.delete_file_record(&params.name, params.delete_nodes)
                    .await?,
            )
        }
        "delete_file_nodes" => {
            let params: NameParams = decode(params)?;
            encode(api.delete_file_nodes(&params.name).await?)
        }
        "save_search_document_result" => {
            let params: SaveSearchDocumentResultParams = decode(params)?;
            api.save_search_document_result(
                &params.search_id,
                &params.document_id,
                &params.method,
                &params.text,
                params.chars,
                params.chunks,
                params.ocr_operation_id.as_deref(),
            )
            .await?;
            Ok(Value::Nil)
        }
        "mark_search_document_ingested" => {
            let params: SearchDocumentParams = decode(params)?;
            api.mark_search_document_ingested(&params.search_id, &params.document_id)
                .await?;
            Ok(Value::Nil)
        }
        "mark_search_knowledge_ingested" => {
            let params: SearchIdParams = decode(params)?;
            api.mark_search_knowledge_ingested(&params.search_id)
                .await?;
            Ok(Value::Nil)
        }

        // Already supported by core and useful to exercise sidecar streams.
        "stream_chat" => {
            let params: StreamChatParams = decode(params)?;
            api.stream_chat(params.messages, stream).await?;
            Ok(Value::Nil)
        }
        "start_stt" => {
            let session_id = state
                .stt()
                .start()
                .await
                .map_err(|error| BackendError::new("stt_start", error).retryable(true))?;
            events
                .emit(
                    "stt-state",
                    serde_json::json!({ "sessionId": session_id, "state": "listening" }),
                )
                .map_err(|error| BackendError::new("stt_event", error))?;
            encode(serde_json::json!({ "sessionId": session_id }))
        }
        "stt_push_audio" => {
            let params: PushSttAudioParams = decode(params)?;
            let session_id = params.session_id;
            let stt_events = state
                .stt()
                .push_audio(session_id.clone(), params.pcm)
                .await
                .map_err(|error| BackendError::new("stt_audio", error).retryable(true))?;
            ayla_core::emit_stt_events(events, &session_id, stt_events)
                .map_err(|error| BackendError::new("stt_event", error))?;
            Ok(Value::Nil)
        }
        "stop_stt" => {
            let params: StopSttParams = decode(params)?;
            let session_id = params.session_id;
            let stt_events = state
                .stt()
                .stop(session_id.clone())
                .await
                .map_err(|error| BackendError::new("stt_stop", error))?;
            ayla_core::emit_stt_events(events, &session_id, stt_events)
                .map_err(|error| BackendError::new("stt_event", error))?;
            events
                .emit(
                    "stt-state",
                    serde_json::json!({ "sessionId": session_id, "state": "idle" }),
                )
                .map_err(|error| BackendError::new("stt_event", error))?;
            Ok(Value::Nil)
        }
        "cancel_process" => {
            let params: CancelProcessParams = decode(params)?;
            encode(state.processes().cancel(&params.pid).await)
        }
        "run_ocr" => {
            let params: RunOcrParams = decode(params)?;
            encode(
                api.run_ocr(
                    &params.file_path,
                    params.knowledge_mode.as_deref(),
                    state.foundation().resources(),
                    events,
                    state.processes(),
                )
                .await?,
            )
        }
        "run_code_analysis" => {
            let params: ayla_core::api::CodeAnalysisRequest = decode(params)?;
            encode(
                api.run_code_analysis(params, events, state.processes())
                    .await?,
            )
        }
        "run_search" => {
            let params: RunSearchParams = decode(params)?;
            encode(
                api.run_search(
                    params.query,
                    params.mode,
                    state.foundation().resources(),
                    events,
                    state.processes(),
                )
                .await?,
            )
        }
        "run_quick_extract" => {
            let params: FilePathParams = decode(params)?;
            encode(api.run_quick_extract(&params.file_path).await?)
        }
        "generate_tts" => {
            let params: GenerateTtsParams = decode(params)?;
            let wav = state
                .tts()
                .generate_with_language(params.text, params.language.as_deref())
                .await
                .map_err(|error| BackendError::new("tts_generation", error).retryable(true))?;
            Ok(Value::Binary(wav))
        }
        "stop_tts" => {
            state
                .tts()
                .stop()
                .map_err(|error| BackendError::new("tts_stop", error))?;
            Ok(Value::Nil)
        }
        _ => Err(BackendError::new(
            "method_not_implemented",
            format!("Command '{method}' is not connected to the sidecar dispatcher"),
        )),
    }
}

fn decode<T: DeserializeOwned>(params: Value) -> Result<T, BackendError> {
    rmpv::ext::from_value(params).map_err(|error| {
        BackendError::new(
            "invalid_params",
            format!("Invalid command parameters: {error}"),
        )
    })
}

fn encode<T: Serialize>(value: T) -> Result<Value, BackendError> {
    let bytes = rmp_serde::to_vec_named(&value).map_err(|error| {
        BackendError::new(
            "result_serialization",
            format!("Cannot serialize command result: {error}"),
        )
    })?;
    rmp_serde::from_slice(&bytes).map_err(|error| {
        BackendError::new(
            "result_serialization",
            format!("Cannot convert command result to MessagePack value: {error}"),
        )
    })
}

#[derive(Deserialize)]
struct NameParams {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIdParams {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchIdParams {
    search_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchDocumentParams {
    search_id: String,
    document_id: String,
}

#[derive(Deserialize)]
struct SearchDocumentsParams {
    query: String,
    limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrichPromptParams {
    user_query: String,
    base_prompt: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessTurnParams {
    user_msg: String,
    assistant_msg: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessMessageParams {
    query: String,
    base_prompt: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngestTextParams {
    text: String,
    source: String,
    source_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveChatSessionParams {
    session_id: String,
    title: String,
    messages_json: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveFileRecordParams {
    name: String,
    path: String,
    chars: u32,
    source_type: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteFileRecordParams {
    name: String,
    delete_nodes: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSearchDocumentResultParams {
    search_id: String,
    document_id: String,
    method: String,
    text: String,
    chars: u32,
    chunks: u32,
    #[serde(default)]
    ocr_operation_id: Option<String>,
}

#[derive(Deserialize)]
struct CancelProcessParams {
    pid: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilePathParams {
    file_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunOcrParams {
    file_path: String,
    #[serde(default)]
    knowledge_mode: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactIdParams {
    artifact_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationIdParams {
    operation_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupIdParams {
    backup_id: String,
}

#[derive(Deserialize)]
struct GraphV2PageParams {
    kind: String,
    offset: usize,
    limit: usize,
}

#[derive(Deserialize)]
struct GraphV2PruneParams {
    retention: usize,
}

#[derive(Deserialize)]
struct RunSearchParams {
    query: String,
    mode: Option<String>,
}

#[derive(Deserialize)]
struct StreamChatParams {
    messages: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct GenerateTtsParams {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushSttAudioParams {
    session_id: String,
    #[serde(with = "serde_bytes")]
    pcm: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StopSttParams {
    session_id: String,
}

#[derive(Deserialize)]
struct SetSystemSttEnabledParams {
    enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayla_core::commands::BACKEND_METHODS;
    use ayla_core::{MemoryConfig, NoopEventSink, NoopStreamSink, ResourceLocator};
    use std::sync::Arc;

    fn offline_state() -> BackendState {
        BackendState::offline(
            MemoryConfig::default(),
            ResourceLocator::new(None, "/workspace"),
            Arc::new(NoopEventSink),
        )
    }

    #[test]
    fn camel_case_parameters_decode() {
        let params = rmpv::ext::to_value(serde_json::json!({
            "searchId": "search-1",
            "documentId": "paper-1"
        }))
        .unwrap();
        let decoded: SearchDocumentParams = decode(params).unwrap();
        assert_eq!(decoded.search_id, "search-1");
        assert_eq!(decoded.document_id, "paper-1");

        let params = rmpv::ext::to_value(serde_json::json!({
            "userMsg": "hello",
            "assistantMsg": "hi"
        }))
        .unwrap();
        let decoded: ProcessTurnParams = decode(params).unwrap();
        assert_eq!(decoded.user_msg, "hello");
        assert_eq!(decoded.assistant_msg, "hi");
    }

    #[test]
    fn stt_audio_accepts_messagepack_binary_payloads() {
        let params = Value::Map(vec![
            (Value::from("sessionId"), Value::from("session-1")),
            (Value::from("pcm"), Value::Binary(vec![0, 128, 255, 127])),
        ]);
        let decoded: PushSttAudioParams = decode(params).unwrap();
        assert_eq!(decoded.session_id, "session-1");
        assert_eq!(decoded.pcm, vec![0, 128, 255, 127]);

        let array_params = Value::Map(vec![
            (Value::from("sessionId"), Value::from("session-2")),
            (
                Value::from("pcm"),
                Value::Array(vec![Value::from(0), Value::from(128)]),
            ),
        ]);
        let decoded_array: PushSttAudioParams = decode(array_params).unwrap();
        assert_eq!(decoded_array.pcm, vec![0, 128]);
    }

    #[tokio::test]
    async fn offline_group_a_reads_return_real_core_results() {
        let state = offline_state();
        let ready = dispatch(
            &state,
            "check_db_ready",
            Value::Map(Vec::new()),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap();
        assert_eq!(ready, Value::Boolean(false));

        let files = dispatch(
            &state,
            "get_files",
            Value::Map(Vec::new()),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap();
        assert_eq!(files, Value::Array(Vec::new()));

        let graph = dispatch(
            &state,
            "get_kb_data",
            Value::Map(Vec::new()),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap();
        let graph = graph
            .as_map()
            .expect("graph response must use named fields");
        assert!(graph
            .iter()
            .any(|(key, value)| key.as_str() == Some("nodes") && value.is_array()));
        assert!(graph
            .iter()
            .any(|(key, value)| key.as_str() == Some("edges") && value.is_array()));

        let graph_v2 = dispatch(
            &state,
            "get_graph_v2_data",
            Value::Map(Vec::new()),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap();
        let graph_v2 = graph_v2
            .as_map()
            .expect("graph-v2 response must use named fields");
        assert!(graph_v2.iter().any(|(key, value)| {
            key.as_str() == Some("available") && value == &Value::Boolean(false)
        }));
    }

    #[tokio::test]
    async fn offline_mutation_preserves_structured_backend_error() {
        let state = offline_state();
        let params = rmpv::ext::to_value(serde_json::json!({
            "sessionId": "session-1",
            "title": "Test",
            "messagesJson": "[]"
        }))
        .unwrap();
        let error = dispatch(
            &state,
            "save_chat_session",
            params,
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "db_offline");
        assert_eq!(error.message, "DB offline");
    }

    #[tokio::test]
    async fn offline_tts_uses_binary_result_contract() {
        let state = offline_state();
        let error = dispatch(
            &state,
            "generate_tts",
            rmpv::ext::to_value(serde_json::json!({"text": "hello"})).unwrap(),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "tts_generation");
        assert!(error.retryable);

        let stopped = dispatch(
            &state,
            "stop_tts",
            Value::Map(Vec::new()),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap();
        assert_eq!(stopped, Value::Nil);
    }

    #[tokio::test]
    async fn system_stt_preference_round_trips_through_the_sidecar() {
        let state = offline_state();
        let enabled = dispatch(
            &state,
            "set_system_stt_enabled",
            rmpv::ext::to_value(serde_json::json!({ "enabled": true })).unwrap(),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap();
        let enabled = enabled.as_map().expect("STT status must be an object");
        assert!(enabled.iter().any(|(key, value)| {
            key.as_str() == Some("systemEnabled") && value == &Value::Boolean(true)
        }));

        let status = dispatch(
            &state,
            "get_stt_status",
            Value::Map(Vec::new()),
            &NoopEventSink,
            &NoopStreamSink,
        )
        .await
        .unwrap();
        assert!(status.as_map().unwrap().iter().any(|(key, value)| {
            key.as_str() == Some("systemEnabled") && value == &Value::Boolean(true)
        }));
    }

    #[tokio::test]
    async fn every_advertised_method_has_a_dispatch_route() {
        let state = offline_state();
        for method in BACKEND_METHODS {
            let result =
                dispatch(&state, method, Value::Nil, &NoopEventSink, &NoopStreamSink).await;
            assert!(
                !matches!(result, Err(ref error) if error.code == "method_not_implemented"),
                "{method} is advertised but not dispatched"
            );
        }
    }
}
