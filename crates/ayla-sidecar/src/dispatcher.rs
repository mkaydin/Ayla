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
        "check_db_ready" => encode(api.check_db_ready()),
        "get_kb_data" => encode(api.get_kb_data().await),
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
        "cancel_process" => {
            let params: CancelProcessParams = decode(params)?;
            encode(state.processes().cancel(&params.pid).await)
        }
        "run_ocr" => {
            let params: FilePathParams = decode(params)?;
            encode(
                api.run_ocr(
                    &params.file_path,
                    state.foundation().resources(),
                    events,
                    state.processes(),
                )
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
                .generate(params.text)
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
            ResourceLocator::new(None, "/workspace/src-tauri"),
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
