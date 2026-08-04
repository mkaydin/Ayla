//! Thin Tauri adapters for the shared TTS service.

pub use alice_core::TtsService as TtsState;
use tauri::State;

#[tauri::command]
pub async fn generate_tts(state: State<'_, TtsState>, text: String) -> Result<Vec<u8>, String> {
    state.generate(text).await
}

#[tauri::command]
pub fn stop_tts(state: State<'_, TtsState>) -> Result<(), String> {
    state.stop()
}
