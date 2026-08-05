mod db;
mod event_sink;
mod live2d;
mod memory;
mod processes;
mod tools;
mod tts;

use memory::config::MemoryConfig;
use memory::MemoryManager;
use processes::ProcessRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tts::TtsState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(ProcessRegistry::new())
        .setup(|app| {
            let resource_dir = app.path().resource_dir().ok();
            let resources = alice_core::ResourceLocator::new(
                resource_dir.clone(),
                PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            );
            let events = Arc::new(event_sink::TauriEventSink::new(app.handle().clone()));
            app.manage(TtsState::new(&resources));
            app.manage(alice_core::BackendFoundation::new(resources, events));
            let config = MemoryConfig::default();
            // Register memory state before the webview can issue IPC commands.
            let manager = tauri::async_runtime::block_on(MemoryManager::new(config));
            app.manage(manager);

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            live2d::read_live2d_asset,
            tts::generate_tts,
            tts::stop_tts,
            memory::commands::enrich_prompt,
            memory::commands::process_turn,
            memory::commands::process_message,
            memory::commands::stream_chat,
            memory::commands::get_kb_data,
            memory::commands::analyze_and_merge_kb,
            memory::commands::run_ocr,
            memory::commands::run_search,
            memory::commands::get_search_history,
            memory::commands::get_search_document_content,
            memory::commands::save_search_document_result,
            memory::commands::mark_search_document_ingested,
            memory::commands::mark_search_knowledge_ingested,
            memory::commands::ingest_text,
            memory::commands::run_consolidation,
            memory::commands::cancel_process,
            memory::commands::save_chat_session,
            memory::commands::load_chat_sessions,
            memory::commands::delete_chat_session,
            memory::commands::check_db_ready,
            memory::commands::save_file_record,
            memory::commands::get_files,
            memory::commands::get_document_text,
            memory::commands::delete_file_record,
            memory::commands::delete_file_nodes,
            memory::commands::run_quick_extract,
            memory::commands::search_documents,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
