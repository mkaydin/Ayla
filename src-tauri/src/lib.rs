mod tts;
mod voice;

use tts::TtsState;
use voice::VoiceState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .manage(TtsState {
      engine: std::sync::Mutex::new(tts::TtsEngine::new()),
    })
    .manage(VoiceState::default())
    .invoke_handler(tauri::generate_handler![
      tts::generate_tts,
      voice::start_listening,
      voice::stop_listening
    ])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
