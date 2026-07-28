mod tts;
mod memory;
mod db;
mod tools;
mod processes;

use tauri::Manager;
use tts::TtsState;
use memory::MemoryManager;
use memory::config::MemoryConfig;
use processes::ProcessRegistry;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(TtsState {
            engine: std::sync::Mutex::new(tts::TtsEngine::new()),
        })
        .manage(ProcessRegistry::new())
        .setup(|app| {
            let config = MemoryConfig::default();
            let handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                let manager = MemoryManager::new(config).await;
                handle.manage(manager);
            });

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
            tts::generate_tts,
            tts::stop_tts,
            memory::commands::enrich_prompt,
            memory::commands::process_turn,
            memory::commands::process_message,
            memory::commands::get_kb_data,
            memory::commands::kb_add_node,
            memory::commands::kb_add_edge,
            memory::commands::run_ocr,
            memory::commands::run_search,
            memory::commands::ingest_text,
            memory::commands::run_consolidation,
            memory::commands::cancel_process,
            memory::commands::save_chat_session,
            memory::commands::load_chat_sessions,
            memory::commands::save_file_record,
            memory::commands::get_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
