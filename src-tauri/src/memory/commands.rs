//! Thin Tauri IPC adapters for `ayla_core::BackendApi`.

use crate::event_sink::TauriChannelStreamSink;
use crate::memory::MemoryManager;
use crate::processes::ProcessRegistry;
use ayla_core::{BackendApi, BackendFoundation};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

fn json_response<T: Serialize>(value: T, label: &str) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("{label} response: {error}"))
}

#[tauri::command]
pub async fn enrich_prompt(
    manager: State<'_, MemoryManager>,
    user_query: String,
    base_prompt: String,
) -> Result<String, String> {
    BackendApi::new(manager.inner())
        .enrich_prompt(&user_query, &base_prompt)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn process_turn(
    manager: State<'_, MemoryManager>,
    user_msg: String,
    assistant_msg: String,
) -> Result<(), String> {
    BackendApi::new(manager.inner())
        .process_turn(&user_msg, &assistant_msg)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn process_message(
    manager: State<'_, MemoryManager>,
    query: String,
    base_prompt: String,
) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .process_message(&query, &base_prompt)
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "Message")
}

#[tauri::command]
pub async fn stream_chat(
    manager: State<'_, MemoryManager>,
    messages: Vec<Value>,
    on_event: tauri::ipc::Channel<Value>,
) -> Result<(), String> {
    let stream = TauriChannelStreamSink::new(on_event);
    BackendApi::new(manager.inner())
        .stream_chat(messages, &stream)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_kb_data(manager: State<'_, MemoryManager>) -> Result<Value, String> {
    json_response(BackendApi::new(manager.inner()).get_kb_data().await, "KB")
}

#[tauri::command]
pub async fn analyze_and_merge_kb(
    backend: State<'_, BackendFoundation>,
    manager: State<'_, MemoryManager>,
) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .analyze_and_merge_kb(backend.events())
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "KB cleanup")
}

#[tauri::command]
pub async fn run_ocr(
    backend: State<'_, BackendFoundation>,
    manager: State<'_, MemoryManager>,
    processes: State<'_, ProcessRegistry>,
    file_path: String,
) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .run_ocr(
            &file_path,
            backend.resources(),
            backend.events(),
            processes.inner(),
        )
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "OCR")
}

#[tauri::command]
pub async fn run_search(
    backend: State<'_, BackendFoundation>,
    manager: State<'_, MemoryManager>,
    processes: State<'_, ProcessRegistry>,
    query: String,
    mode: Option<String>,
) -> Result<Value, String> {
    BackendApi::new(manager.inner())
        .run_search(
            query,
            mode,
            backend.resources(),
            backend.events(),
            processes.inner(),
        )
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_search_history(manager: State<'_, MemoryManager>) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .get_search_history()
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "Search history")
}

#[tauri::command]
pub async fn get_search_document_content(
    manager: State<'_, MemoryManager>,
    search_id: String,
    document_id: String,
) -> Result<Value, String> {
    BackendApi::new(manager.inner())
        .get_search_document_content(&search_id, &document_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_search_document_result(
    manager: State<'_, MemoryManager>,
    search_id: String,
    document_id: String,
    method: String,
    text: String,
    chars: u32,
    chunks: u32,
) -> Result<(), String> {
    BackendApi::new(manager.inner())
        .save_search_document_result(&search_id, &document_id, &method, &text, chars, chunks)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn mark_search_document_ingested(
    manager: State<'_, MemoryManager>,
    search_id: String,
    document_id: String,
) -> Result<(), String> {
    BackendApi::new(manager.inner())
        .mark_search_document_ingested(&search_id, &document_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn mark_search_knowledge_ingested(
    manager: State<'_, MemoryManager>,
    search_id: String,
) -> Result<(), String> {
    BackendApi::new(manager.inner())
        .mark_search_knowledge_ingested(&search_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn cancel_process(
    processes: State<'_, ProcessRegistry>,
    pid: String,
) -> Result<bool, String> {
    log::info!("Cancelling process {}", pid);
    Ok(processes.cancel(&pid).await)
}

#[tauri::command]
pub async fn ingest_text(
    backend: State<'_, BackendFoundation>,
    manager: State<'_, MemoryManager>,
    text: String,
    source: String,
    source_type: String,
) -> Result<String, String> {
    BackendApi::new(manager.inner())
        .ingest_text(text, source, source_type, backend.events())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_consolidation(manager: State<'_, MemoryManager>) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .run_consolidation()
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "Consolidation")
}

#[tauri::command]
pub async fn save_chat_session(
    manager: State<'_, MemoryManager>,
    session_id: String,
    title: String,
    messages_json: String,
) -> Result<(), String> {
    BackendApi::new(manager.inner())
        .save_chat_session(&session_id, &title, &messages_json)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn load_chat_sessions(manager: State<'_, MemoryManager>) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .load_chat_sessions()
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "Chat sessions")
}

#[tauri::command]
pub async fn delete_chat_session(
    manager: State<'_, MemoryManager>,
    session_id: String,
) -> Result<(), String> {
    BackendApi::new(manager.inner())
        .delete_chat_session(&session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn check_db_ready(manager: State<'_, MemoryManager>) -> Result<bool, String> {
    Ok(BackendApi::new(manager.inner()).check_db_ready())
}

#[tauri::command]
pub async fn save_file_record(
    manager: State<'_, MemoryManager>,
    name: String,
    path: String,
    chars: u32,
    source_type: String,
    text: String,
) -> Result<(), String> {
    BackendApi::new(manager.inner())
        .save_file_record(name, path, chars, source_type, text)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_files(manager: State<'_, MemoryManager>) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .get_files()
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "Files")
}

#[tauri::command]
pub async fn get_document_text(
    manager: State<'_, MemoryManager>,
    name: String,
) -> Result<String, String> {
    Ok(BackendApi::new(manager.inner())
        .get_document_text(&name)
        .await)
}

#[tauri::command]
pub async fn delete_file_record(
    manager: State<'_, MemoryManager>,
    name: String,
    delete_nodes: bool,
) -> Result<String, String> {
    BackendApi::new(manager.inner())
        .delete_file_record(&name, delete_nodes)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_file_nodes(
    manager: State<'_, MemoryManager>,
    name: String,
) -> Result<String, String> {
    BackendApi::new(manager.inner())
        .delete_file_nodes(&name)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_quick_extract(
    manager: State<'_, MemoryManager>,
    file_path: String,
) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .run_quick_extract(&file_path)
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "Quick Extract")
}

#[tauri::command]
pub async fn search_documents(
    manager: State<'_, MemoryManager>,
    query: String,
    limit: u32,
) -> Result<Value, String> {
    let result = BackendApi::new(manager.inner())
        .search_documents(&query, limit)
        .await
        .map_err(|error| error.to_string())?;
    json_response(result, "Document search")
}
