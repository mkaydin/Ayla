// TTS engine wrapping sherpa-onnx Piper VITS (amy model).
//
// The TtsEngine is created once at startup and held behind a Mutex so it can
// be shared across Tauri command invocations safely.

use std::sync::Mutex;

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
    /// Load the Piper amy medium model.
    pub fn new() -> Self {
        // Cargo.toml lives in src-tauri/; go up one level to project root.
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let model = base
            .join("tts/tts_piper_amy_low/en_US-amy-low.onnx")
            .to_str()
            .unwrap()
            .to_string();
        let tokens = base
            .join("tts/tts_piper_amy_low/tokens.txt")
            .to_str()
            .unwrap()
            .to_string();
        let data_dir = base
            .join("tts/tts_piper_amy_low/espeak-ng-data")
            .to_str()
            .unwrap()
            .to_string();

        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                vits: OfflineTtsVitsModelConfig {
                    model: Some(model),
                    tokens: Some(tokens),
                    data_dir: Some(data_dir),
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

        let tts = OfflineTts::create(&config).expect("Failed to load Piper amy TTS model");
        log::info!("TTS loaded  sample_rate={}  speakers={}", tts.sample_rate(), tts.num_speakers());
        TtsEngine { tts }
    }

    /// Synthesize text into WAV (PCM 16-bit mono) bytes.
    pub fn generate(&self, text: &str) -> Vec<u8> {
        let gen = GenerationConfig { sid: 0, speed: 1.10, ..Default::default() };

        let audio: GeneratedAudio = self.tts
            .generate_with_config(text, &gen, None::<fn(&[f32], f32) -> bool>)
            .expect("TTS generation failed");

        samples_to_wav(audio.samples(), audio.sample_rate() as u32)
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
    pub engine: Mutex<TtsEngine>,
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn generate_tts(state: tauri::State<'_, TtsState>, text: String) -> Vec<u8> {
    let engine = state.engine.lock().unwrap();
    engine.generate(&text)
}
