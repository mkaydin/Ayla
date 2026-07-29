// TTS engine wrapping sherpa-onnx Piper VITS (amy model).
//
// The TtsEngine is created once at startup and held behind a Mutex so it can
// be shared across Tauri command invocations safely.

use std::sync::{Arc, Mutex};

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsVitsModelConfig, GeneratedAudio,
};

// ---------------------------------------------------------------------------
// TTS engine
// ---------------------------------------------------------------------------

pub struct TtsEngine {
    tts: OfflineTts,
}

impl TtsEngine {
    /// Load the Piper amy model. Returns `None` (and logs a warning) if the model files
    /// are missing, instead of panicking — so the rest of the app can still start and the
    /// TTS command degrades to a friendly error (issue R10).
    pub fn new() -> Option<Self> {
        // Cargo.toml lives in src-tauri/; go up one level to project root.
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let model = base.join("tts/tts_piper_amy_low/en_US-amy-low.onnx");
        let tokens = base.join("tts/tts_piper_amy_low/tokens.txt");
        let data_dir = base.join("tts/tts_piper_amy_low/espeak-ng-data");

        if !model.exists() {
            log::warn!("TTS model not found at {} — TTS disabled. Download per README.", model.display());
            return None;
        }

        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                vits: OfflineTtsVitsModelConfig {
                    model: Some(model.to_string_lossy().into_owned()),
                    tokens: Some(tokens.to_string_lossy().into_owned()),
                    data_dir: Some(data_dir.to_string_lossy().into_owned()),
                    noise_scale: 0.667,
                    noise_scale_w: 0.8,
                    length_scale: 1.0,
                    ..Default::default()
                },
                num_threads: 2,
                debug: cfg!(debug_assertions),
                ..Default::default()
            },
            ..Default::default()
        };

        match OfflineTts::create(&config) {
            Some(tts) => {
                log::info!("TTS loaded  sample_rate={}  speakers={}", tts.sample_rate(), tts.num_speakers());
                Some(TtsEngine { tts })
            }
            None => {
                log::warn!("Failed to initialize Piper TTS ( OfflineTts::create returned None ) — TTS disabled.");
                None
            }
        }
    }

    /// Synthesize text into WAV (PCM 16-bit mono) bytes.
    pub fn generate(&self, text: &str) -> Result<Vec<u8>, String> {
        let gen = GenerationConfig { sid: 0, speed: 1.10, ..Default::default() };

        let audio: GeneratedAudio = self.tts
            .generate_with_config(text, &gen, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| "TTS generation returned no audio".to_string())?;

        Ok(samples_to_wav(audio.samples(), audio.sample_rate() as u32))
    }
}

// ---------------------------------------------------------------------------
// WAV encoding (manual — sherpa-onnx only writes to file)
// ---------------------------------------------------------------------------

fn samples_to_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len() as u32;
    let data_size = num_samples * 2;              // 16-bit mono
    let file_size = 36 + data_size;

    let mut buf = Vec::with_capacity(44 + data_size as usize);

    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    // fmt  chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());   // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes());    // PCM
    buf.extend_from_slice(&1u16.to_le_bytes());    // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes());    // block align
    buf.extend_from_slice(&16u16.to_le_bytes());   // bits per sample

    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        let i16 = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
        buf.extend_from_slice(&i16.to_le_bytes());
    }

    buf
}

// ---------------------------------------------------------------------------
// Tauri managed state
// ---------------------------------------------------------------------------

pub struct TtsState {
    /// `None` when the model couldn't be loaded at startup (TTS disabled, app still runs).
    /// `Arc` so a blocking synthesis thread can take its own lock without the async command
    /// holding the Tauri State borrow (issues R11/R12).
    pub engine: Arc<Mutex<Option<TtsEngine>>>,
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

/// Synthesize `text` to WAV bytes. Runs synthesis on a blocking thread so it doesn't stall
/// the async Tauri runtime, and recovers from a poisoned mutex (issues R11/R12).
#[tauri::command]
pub async fn generate_tts(state: tauri::State<'_, TtsState>, text: String) -> Result<Vec<u8>, String> {
    let engine = Arc::clone(&state.engine);
    // CPU-bound synthesis off the async runtime worker thread (issue R11).
    tokio::task::spawn_blocking(move || {
        // Recover from a poisoned mutex rather than panicking (issue R12).
        let guard = engine.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(eng) => eng.generate(&text),
            None => Err("TTS engine unavailable (model not loaded)".to_string()),
        }
    })
    .await
    .map_err(|e| format!("TTS task panicked: {}", e))?
}

#[tauri::command]
pub fn stop_tts() -> Result<(), String> {
    // No-op placeholder for API compatibility (issue R45). Actual playback stop is handled
    // client-side in tts-player.js.
    Ok(())
}
