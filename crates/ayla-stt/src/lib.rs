//! Offline Omnilingual ASR and Silero VAD for Ayla.
//!
//! Audio capture is intentionally host-neutral: the desktop renderer captures
//! microphone audio, resamples it to 16 kHz mono, and submits bounded PCM16
//! chunks. No Python runtime or external STT process is used.

use serde::Serialize;
use sherpa_onnx::{
    OfflineModelConfig, OfflineOmnilingualAsrCtcModelConfig, OfflineRecognizer,
    OfflineRecognizerConfig, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const SAMPLE_RATE: i32 = 16_000;
pub const VAD_WINDOW_SIZE: usize = 512;
pub const MAX_PCM_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct SttModel {
    pub model: PathBuf,
    pub tokens: PathBuf,
    pub vad: PathBuf,
}

impl SttModel {
    pub fn missing_files(&self) -> Vec<PathBuf> {
        [&self.model, &self.tokens, &self.vad]
            .into_iter()
            .filter(|path| !path.is_file())
            .cloned()
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.missing_files().is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct SttOptions {
    pub threads: i32,
    pub vad_threshold: f32,
    pub min_silence_seconds: f32,
    pub min_speech_seconds: f32,
    pub max_speech_seconds: f32,
    pub partial_interval_seconds: f32,
}

impl Default for SttOptions {
    fn default() -> Self {
        Self {
            threads: 4,
            vad_threshold: 0.5,
            min_silence_seconds: 0.5,
            min_speech_seconds: 0.25,
            max_speech_seconds: 30.0,
            partial_interval_seconds: 0.5,
        }
    }
}

impl SttOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=32).contains(&self.threads) {
            return Err("STT threads must be between 1 and 32".into());
        }
        if !(0.0..=1.0).contains(&self.vad_threshold) {
            return Err("STT VAD threshold must be between 0 and 1".into());
        }
        if !(0.1..=5.0).contains(&self.min_silence_seconds) {
            return Err("STT minimum silence must be between 0.1 and 5 seconds".into());
        }
        if !(0.05..=5.0).contains(&self.min_speech_seconds) {
            return Err("STT minimum speech must be between 0.05 and 5 seconds".into());
        }
        if !(1.0..=60.0).contains(&self.max_speech_seconds) {
            return Err("STT maximum speech must be between 1 and 60 seconds".into());
        }
        if !(0.0..=10.0).contains(&self.partial_interval_seconds) {
            return Err("STT partial interval must be between 0 and 10 seconds".into());
        }
        Ok(())
    }
}

pub struct SttEngine {
    recognizer: OfflineRecognizer,
    model: SttModel,
    options: SttOptions,
}

impl SttEngine {
    pub fn load(model: SttModel, options: SttOptions) -> Result<Self, String> {
        options.validate()?;
        let missing = model.missing_files();
        if !missing.is_empty() {
            return Err(format!(
                "STT model is incomplete (missing: {})",
                missing
                    .iter()
                    .map(|path| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let config = OfflineRecognizerConfig {
            model_config: OfflineModelConfig {
                omnilingual: OfflineOmnilingualAsrCtcModelConfig {
                    model: Some(path_string(&model.model)),
                },
                tokens: Some(path_string(&model.tokens)),
                num_threads: options.threads,
                debug: false,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| "sherpa-onnx rejected the Omnilingual ASR configuration".to_string())?;
        Ok(Self {
            recognizer,
            model,
            options,
        })
    }

    pub fn decode(&self, samples: &[f32]) -> Result<String, String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE, samples);
        self.recognizer.decode(&stream);
        Ok(stream
            .get_result()
            .ok_or_else(|| "Omnilingual ASR returned no result".to_string())?
            .text
            .trim()
            .to_string())
    }

    pub fn start_session(&self) -> Result<SttSession, String> {
        let vad_config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(path_string(&self.model.vad)),
                threshold: self.options.vad_threshold,
                min_silence_duration: self.options.min_silence_seconds,
                min_speech_duration: self.options.min_speech_seconds,
                window_size: VAD_WINDOW_SIZE as i32,
                max_speech_duration: self.options.max_speech_seconds,
            },
            sample_rate: SAMPLE_RATE,
            num_threads: 1,
            provider: Some("cpu".into()),
            debug: false,
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&vad_config, 60.0)
            .ok_or_else(|| "sherpa-onnx failed to initialize Silero VAD".to_string())?;
        Ok(SttSession {
            id: Uuid::new_v4().to_string(),
            vad,
            pending: Vec::new(),
            partial_audio: Vec::new(),
            speech_started: false,
            next_partial_sample: usize::MAX,
            last_partial: String::new(),
            sequence: 0,
            partial_interval_samples: (self.options.partial_interval_seconds * SAMPLE_RATE as f32)
                as usize,
        })
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SttEvent {
    Level {
        rms: f32,
        peak: f32,
    },
    SpeechStarted,
    Partial {
        sequence: u64,
        text: String,
    },
    Final {
        sequence: u64,
        raw_text: String,
        text: String,
        repair_status: &'static str,
    },
}

pub struct SttSession {
    id: String,
    vad: VoiceActivityDetector,
    pending: Vec<f32>,
    partial_audio: Vec<f32>,
    speech_started: bool,
    next_partial_sample: usize,
    last_partial: String,
    sequence: u64,
    partial_interval_samples: usize,
}

impl SttSession {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn push_pcm16(&mut self, engine: &SttEngine, pcm: &[u8]) -> Result<Vec<SttEvent>, String> {
        let samples = pcm16_to_f32(pcm)?;
        let mut events = vec![level_event(&samples)];
        self.partial_audio.extend_from_slice(&samples);
        self.pending.extend_from_slice(&samples);

        let mut consumed = 0;
        while consumed + VAD_WINDOW_SIZE <= self.pending.len() {
            self.vad
                .accept_waveform(&self.pending[consumed..consumed + VAD_WINDOW_SIZE]);
            consumed += VAD_WINDOW_SIZE;
            if !self.speech_started && self.vad.detected() {
                self.speech_started = true;
                self.next_partial_sample = self
                    .partial_audio
                    .len()
                    .saturating_add(self.partial_interval_samples);
                events.push(SttEvent::SpeechStarted);
            }
        }
        if consumed > 0 {
            self.pending.drain(..consumed);
        }

        if self.speech_started
            && self.partial_interval_samples > 0
            && self.partial_audio.len() >= self.next_partial_sample
        {
            let text = engine.decode(&self.partial_audio)?;
            if !text.is_empty() && text != self.last_partial {
                self.last_partial = text.clone();
                events.push(SttEvent::Partial {
                    sequence: self.sequence,
                    text,
                });
            }
            self.next_partial_sample = self
                .partial_audio
                .len()
                .saturating_add(self.partial_interval_samples);
        }
        self.drain_finals(engine, &mut events)?;
        Ok(events)
    }

    pub fn finish(mut self, engine: &SttEngine) -> Result<Vec<SttEvent>, String> {
        if !self.pending.is_empty() {
            let padding = VAD_WINDOW_SIZE - self.pending.len();
            self.pending.resize(self.pending.len() + padding, 0.0);
            self.vad.accept_waveform(&self.pending);
            self.pending.clear();
        }
        self.vad.flush();
        let mut events = Vec::new();
        self.drain_finals(engine, &mut events)?;
        Ok(events)
    }

    fn drain_finals(
        &mut self,
        engine: &SttEngine,
        events: &mut Vec<SttEvent>,
    ) -> Result<(), String> {
        while let Some(segment) = self.vad.front() {
            let samples = segment.samples().to_vec();
            drop(segment);
            self.vad.pop();
            let text = engine.decode(&samples)?;
            if !text.is_empty() {
                events.push(SttEvent::Final {
                    sequence: self.sequence,
                    raw_text: text.clone(),
                    text,
                    repair_status: "off",
                });
                self.sequence += 1;
            }
            self.partial_audio.clear();
            self.speech_started = false;
            self.next_partial_sample = usize::MAX;
            self.last_partial.clear();
        }
        Ok(())
    }
}

pub fn pcm16_to_f32(pcm: &[u8]) -> Result<Vec<f32>, String> {
    if pcm.len() > MAX_PCM_CHUNK_BYTES {
        return Err(format!(
            "STT PCM chunk exceeds the {} byte limit",
            MAX_PCM_CHUNK_BYTES
        ));
    }
    if pcm.len() % 2 != 0 {
        return Err("STT PCM16 chunk has an odd byte length".into());
    }
    Ok(pcm
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32768.0)
        .collect())
}

fn level_event(samples: &[f32]) -> SttEvent {
    if samples.is_empty() {
        return SttEvent::Level {
            rms: 0.0,
            peak: 0.0,
        };
    }
    let mut energy = 0.0_f32;
    let mut peak = 0.0_f32;
    for sample in samples {
        energy += sample * sample;
        peak = peak.max(sample.abs());
    }
    SttEvent::Level {
        rms: (energy / samples.len() as f32).sqrt(),
        peak,
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_conversion_handles_extremes() {
        let samples = pcm16_to_f32(&[
            0x00, 0x80, // -32768
            0x00, 0x00, // 0
            0xff, 0x7f, // 32767
        ])
        .unwrap();
        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!(samples[2] > 0.999);
    }

    #[test]
    fn pcm16_conversion_rejects_unbounded_or_malformed_chunks() {
        assert!(pcm16_to_f32(&[1]).is_err());
        assert!(pcm16_to_f32(&vec![0; MAX_PCM_CHUNK_BYTES + 2]).is_err());
    }

    #[test]
    fn validates_vad_and_decode_options() {
        assert!(SttOptions::default().validate().is_ok());
        assert!(SttOptions {
            vad_threshold: 1.1,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn missing_model_files_are_detected_without_native_initialization() {
        let model = SttModel {
            model: "/missing/model.onnx".into(),
            tokens: "/missing/tokens.txt".into(),
            vad: "/missing/vad.onnx".into(),
        };
        assert_eq!(model.missing_files().len(), 3);
        assert!(!model.is_complete());
    }

    #[test]
    fn level_event_reports_rms_and_peak() {
        assert_eq!(
            level_event(&[-0.5, 0.5]),
            SttEvent::Level {
                rms: 0.5,
                peak: 0.5
            }
        );
    }
}
