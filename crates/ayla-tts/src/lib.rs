//! Host-neutral offline text-to-speech engines used by every Ayla desktop host.
//!
//! or playback code. Hosts resolve model resources, while `ayla-core` owns
//! orchestration and cancellation.

use serde_json::Value;
use sherpa_onnx::{
    GeneratedAudio, GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsSupertonicModelConfig, OfflineTtsVitsModelConfig,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;

pub const SUPERTONIC_VOICE: &str = "F1";
pub const SUPERTONIC_LANGUAGES: [&str; 2] = ["tr", "en"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TtsLanguage {
    Turkish,
    English,
}

impl TtsLanguage {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tr" | "turkish" | "türkçe" => Ok(Self::Turkish),
            "en" | "english" => Ok(Self::English),
            _ => Err("TTS language must be 'tr' (Turkish) or 'en' (English)".into()),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Turkish => "tr",
            Self::English => "en",
        }
    }
}

const SUPERTONIC_FILES: [&str; 7] = [
    "duration_predictor.int8.onnx",
    "text_encoder.int8.onnx",
    "vector_estimator.int8.onnx",
    "vocoder.int8.onnx",
    "tts.json",
    "unicode_indexer.bin",
    "voice.bin",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineKind {
    Supertonic,
    Piper,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supertonic => "supertonic",
            Self::Piper => "piper",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SynthesisOptions {
    pub speed: f32,
    pub num_steps: i32,
}

impl Default for SynthesisOptions {
    fn default() -> Self {
        Self {
            speed: 1.0,
            num_steps: 8,
        }
    }
}

impl SynthesisOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.7..=2.0).contains(&self.speed) {
            return Err("TTS speed must be between 0.7 and 2.0".into());
        }
        if !(1..=100).contains(&self.num_steps) {
            return Err("TTS steps must be between 1 and 100".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SupertonicModel {
    pub directory: PathBuf,
}

impl SupertonicModel {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn missing_files(&self) -> Vec<&'static str> {
        SUPERTONIC_FILES
            .iter()
            .copied()
            .filter(|name| !self.directory.join(name).is_file())
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.missing_files().is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct PiperModel {
    pub model: PathBuf,
    pub tokens: PathBuf,
    pub data_dir: PathBuf,
}

impl PiperModel {
    pub fn is_complete(&self) -> bool {
        self.model.is_file() && self.tokens.is_file() && self.data_dir.is_dir()
    }
}

#[derive(Clone, Debug)]
pub enum ModelConfig {
    Supertonic(SupertonicModel),
    Piper(PiperModel),
}

pub struct TtsEngine {
    tts: OfflineTts,
    kind: EngineKind,
    voice: &'static str,
}

impl TtsEngine {
    pub fn load(model: ModelConfig, threads: i32) -> Result<Self, String> {
        let threads = threads.clamp(1, 32);
        let (kind, voice, model_config) = match model {
            ModelConfig::Supertonic(model) => {
                let missing = model.missing_files();
                if !missing.is_empty() {
                    return Err(format!(
                        "Supertonic model is incomplete at {} (missing: {})",
                        model.directory.display(),
                        missing.join(", ")
                    ));
                }
                let file =
                    |name: &str| Some(model.directory.join(name).to_string_lossy().into_owned());
                (
                    EngineKind::Supertonic,
                    SUPERTONIC_VOICE,
                    OfflineTtsModelConfig {
                        supertonic: OfflineTtsSupertonicModelConfig {
                            duration_predictor: file("duration_predictor.int8.onnx"),
                            text_encoder: file("text_encoder.int8.onnx"),
                            vector_estimator: file("vector_estimator.int8.onnx"),
                            vocoder: file("vocoder.int8.onnx"),
                            tts_json: file("tts.json"),
                            unicode_indexer: file("unicode_indexer.bin"),
                            voice_style: file("voice.bin"),
                        },
                        num_threads: threads,
                        debug: false,
                        provider: Some("cpu".into()),
                        ..Default::default()
                    },
                )
            }
            ModelConfig::Piper(model) => {
                if !model.is_complete() {
                    return Err(format!(
                        "Piper model is incomplete (model: {}, tokens: {}, data: {})",
                        model.model.display(),
                        model.tokens.display(),
                        model.data_dir.display()
                    ));
                }
                (
                    EngineKind::Piper,
                    "amy-low",
                    OfflineTtsModelConfig {
                        vits: OfflineTtsVitsModelConfig {
                            model: Some(model.model.to_string_lossy().into_owned()),
                            tokens: Some(model.tokens.to_string_lossy().into_owned()),
                            data_dir: Some(model.data_dir.to_string_lossy().into_owned()),
                            noise_scale: 0.667,
                            noise_scale_w: 0.8,
                            length_scale: 1.0,
                            ..Default::default()
                        },
                        num_threads: threads,
                        debug: false,
                        provider: Some("cpu".into()),
                        ..Default::default()
                    },
                )
            }
        };

        let config = OfflineTtsConfig {
            model: model_config,
            ..Default::default()
        };
        let tts = OfflineTts::create(&config).ok_or_else(|| {
            format!(
                "sherpa-onnx rejected the {} TTS configuration",
                kind.as_str()
            )
        })?;

        Ok(Self { tts, kind, voice })
    }

    pub fn kind(&self) -> EngineKind {
        self.kind
    }

    pub fn voice(&self) -> &'static str {
        self.voice
    }

    pub fn supports_language(&self, language: TtsLanguage) -> bool {
        self.kind == EngineKind::Supertonic || language == TtsLanguage::English
    }

    pub fn sample_rate(&self) -> i32 {
        self.tts.sample_rate()
    }

    /// Synthesize PCM16 mono WAV bytes. Cancellation is checked by the native
    /// progress callback, so stopping a request also stops its CPU work.
    pub fn generate(
        &self,
        text: &str,
        language: TtsLanguage,
        options: &SynthesisOptions,
        cancellation_epoch: Arc<AtomicU64>,
        request_epoch: u64,
    ) -> Result<Vec<u8>, String> {
        options.validate()?;
        if !self.supports_language(language) {
            return Err(format!(
                "The {} TTS engine does not support language '{}'",
                self.kind.as_str(),
                language.code()
            ));
        }
        let prepared = prepare_text(text, self.kind, language)?;
        let mut extra = HashMap::<String, Value>::new();
        if self.kind == EngineKind::Supertonic {
            extra.insert("lang".into(), Value::String(language.code().into()));
        }
        let config = GenerationConfig {
            sid: 0,
            speed: options.speed,
            num_steps: options.num_steps,
            extra: (!extra.is_empty()).then_some(extra),
            ..Default::default()
        };
        let callback_epoch = Arc::clone(&cancellation_epoch);
        let audio: GeneratedAudio = self
            .tts
            .generate_with_config(
                &prepared,
                &config,
                Some(move |_: &[f32], _: f32| {
                    callback_epoch.load(Ordering::Acquire) == request_epoch
                }),
            )
            .ok_or_else(|| "TTS generation returned no audio".to_string())?;

        if cancellation_epoch.load(Ordering::Acquire) != request_epoch {
            return Err("TTS generation cancelled".into());
        }
        Ok(samples_to_wav(audio.samples(), audio.sample_rate() as u32))
    }
}

pub fn prepare_turkish_text(text: &str) -> Result<String, String> {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return Err("TTS text must not be empty".into());
    }
    Ok(cleaned.nfkd().collect())
}

fn prepare_text(text: &str, kind: EngineKind, language: TtsLanguage) -> Result<String, String> {
    if kind == EngineKind::Supertonic && language == TtsLanguage::Turkish {
        return prepare_turkish_text(text);
    }
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        Err("TTS text must not be empty".into())
    } else {
        Ok(cleaned)
    }
}

fn samples_to_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len() as u32;
    let data_size = num_samples * 2;
    let file_size = 36 + data_size;
    let mut buffer = Vec::with_capacity(44 + data_size as usize);
    buffer.extend_from_slice(b"RIFF");
    buffer.extend_from_slice(&file_size.to_le_bytes());
    buffer.extend_from_slice(b"WAVEfmt ");
    buffer.extend_from_slice(&16_u32.to_le_bytes());
    buffer.extend_from_slice(&1_u16.to_le_bytes());
    buffer.extend_from_slice(&1_u16.to_le_bytes());
    buffer.extend_from_slice(&sample_rate.to_le_bytes());
    buffer.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    buffer.extend_from_slice(&2_u16.to_le_bytes());
    buffer.extend_from_slice(&16_u16.to_le_bytes());
    buffer.extend_from_slice(b"data");
    buffer.extend_from_slice(&data_size.to_le_bytes());
    for &sample in samples {
        let pcm = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        buffer.extend_from_slice(&pcm.to_le_bytes());
    }
    buffer
}

pub fn path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_turkish_diacritics_and_whitespace() {
        let prepared = prepare_turkish_text("  Şişli'deki\n çiçekçi  ").unwrap();
        assert_eq!(
            prepared,
            "S\u{327}is\u{327}li'deki c\u{327}ic\u{327}ekc\u{327}i"
        );
    }

    #[test]
    fn rejects_empty_text() {
        assert_eq!(
            prepare_turkish_text(" \n\t ").unwrap_err(),
            "TTS text must not be empty"
        );
    }

    #[test]
    fn parses_supported_language_names_and_rejects_unknown_values() {
        assert_eq!(TtsLanguage::parse("tr").unwrap(), TtsLanguage::Turkish);
        assert_eq!(TtsLanguage::parse("English").unwrap(), TtsLanguage::English);
        assert!(TtsLanguage::parse("auto").is_err());
    }

    #[test]
    fn english_preparation_preserves_english_text() {
        assert_eq!(
            prepare_text(
                "  Hello,   this is Ayla.  ",
                EngineKind::Supertonic,
                TtsLanguage::English
            )
            .unwrap(),
            "Hello, this is Ayla."
        );
    }

    #[test]
    fn validates_generation_ranges() {
        assert!(SynthesisOptions {
            speed: 0.69,
            num_steps: 8
        }
        .validate()
        .is_err());
        assert!(SynthesisOptions {
            speed: 1.0,
            num_steps: 101
        }
        .validate()
        .is_err());
        assert!(SynthesisOptions::default().validate().is_ok());
    }

    #[test]
    fn detects_incomplete_supertonic_model() {
        let model = SupertonicModel::new("/definitely/missing/supertonic");
        assert_eq!(model.missing_files(), SUPERTONIC_FILES);
    }

    #[test]
    fn wav_encoder_writes_pcm_header_and_samples() {
        let wav = samples_to_wav(&[-1.0, 0.0, 1.0], 44_100);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 50);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 44_100);
    }
}
