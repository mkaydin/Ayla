//! Application-level STT lifecycle and resource orchestration.

use crate::config::SttConfig;
use crate::{EventSink, ResourceLocator};
use ayla_stt::{SttEngine, SttEvent, SttModel, SttOptions, SttSession};
use serde::Serialize;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MODEL_DIRECTORY: &str = "sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-int8-2025-11-12";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttStatus {
    pub available: bool,
    pub loaded: bool,
    pub listening: bool,
    pub system_enabled: bool,
    pub model: String,
    pub provider: String,
    pub language_mode: String,
    pub supported_languages: Vec<String>,
    pub capture_backend: String,
    pub sample_rate: i32,
    pub repair_mode: String,
    pub reason: Option<String>,
}

struct SttRuntime {
    engine: Option<SttEngine>,
    session: Option<SttSession>,
}

pub struct SttService {
    runtime: Arc<Mutex<SttRuntime>>,
    model: Option<SttModel>,
    options: SttOptions,
    loaded: Arc<AtomicBool>,
    listening: Arc<AtomicBool>,
    system_enabled: Arc<AtomicBool>,
    unavailable_reason: Option<String>,
}

impl SttService {
    pub fn new(resources: &ResourceLocator) -> Self {
        Self::with_config(resources, SttConfig::default())
    }

    pub fn with_config(resources: &ResourceLocator, config: SttConfig) -> Self {
        let root = config
            .model_dir
            .unwrap_or_else(|| resources.resolve_resource("models/stt", "models/stt"));
        let model_root = root.join(MODEL_DIRECTORY);
        let model = SttModel {
            model: model_root.join("model.int8.onnx"),
            tokens: model_root.join("tokens.txt"),
            vad: root.join("silero_vad.onnx"),
        };
        let missing = model.missing_files();
        let unavailable_reason = (!missing.is_empty()).then(|| {
            format!(
                "STT model is incomplete (missing: {})",
                missing
                    .iter()
                    .map(|path| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
        Self {
            runtime: Arc::new(Mutex::new(SttRuntime {
                engine: None,
                session: None,
            })),
            model: unavailable_reason.is_none().then_some(model),
            options: SttOptions {
                threads: config.threads,
                vad_threshold: config.vad_threshold,
                min_silence_seconds: config.min_silence_seconds,
                partial_interval_seconds: config.partial_interval_seconds,
                ..Default::default()
            },
            loaded: Arc::new(AtomicBool::new(false)),
            listening: Arc::new(AtomicBool::new(false)),
            system_enabled: Arc::new(AtomicBool::new(config.system_enabled)),
            unavailable_reason,
        }
    }

    pub fn disabled() -> Self {
        Self {
            runtime: Arc::new(Mutex::new(SttRuntime {
                engine: None,
                session: None,
            })),
            model: None,
            options: SttOptions::default(),
            loaded: Arc::new(AtomicBool::new(false)),
            listening: Arc::new(AtomicBool::new(false)),
            system_enabled: Arc::new(AtomicBool::new(false)),
            unavailable_reason: Some("STT disabled in offline backend mode".into()),
        }
    }

    pub fn status(&self) -> SttStatus {
        SttStatus {
            available: self.model.is_some(),
            loaded: self.loaded.load(Ordering::Acquire),
            listening: self.listening.load(Ordering::Acquire),
            system_enabled: self.system_enabled.load(Ordering::Acquire),
            model: "omni-300m-int8".into(),
            provider: "cpu".into(),
            language_mode: "auto".into(),
            supported_languages: vec!["tr".into(), "en".into()],
            capture_backend: "renderer".into(),
            sample_rate: ayla_stt::SAMPLE_RATE,
            repair_mode: "off".into(),
            reason: self.unavailable_reason.clone(),
        }
    }

    /// Updates the system-command listening preference. This deliberately
    /// does not start a microphone session yet: system commands are a future
    /// consumer. Explicit chat dictation sessions use `start` regardless of
    /// this flag.
    pub fn set_system_enabled(&self, enabled: bool) -> SttStatus {
        self.system_enabled.store(enabled, Ordering::Release);
        self.status()
    }

    /// Starts an explicit, user-initiated capture session. Manual chat
    /// dictation is intentionally independent of `system_enabled`.
    pub async fn start(&self) -> Result<String, String> {
        let runtime = Arc::clone(&self.runtime);
        let model = self.model.clone().ok_or_else(|| {
            self.unavailable_reason
                .clone()
                .unwrap_or_else(|| "STT unavailable".into())
        })?;
        let options = self.options.clone();
        let loaded = Arc::clone(&self.loaded);
        let listening = Arc::clone(&self.listening);
        tokio::task::spawn_blocking(move || {
            let mut runtime = runtime.lock().unwrap_or_else(|error| error.into_inner());
            if runtime.session.is_some() {
                return Err("An STT session is already active".into());
            }
            if runtime.engine.is_none() {
                runtime.engine = Some(SttEngine::load(model, options)?);
                loaded.store(true, Ordering::Release);
            }
            let session = runtime
                .engine
                .as_ref()
                .expect("STT engine initialized")
                .start_session()?;
            let session_id = session.id().to_string();
            runtime.session = Some(session);
            listening.store(true, Ordering::Release);
            Ok(session_id)
        })
        .await
        .map_err(|error| format!("STT start task panicked: {error}"))?
    }

    pub async fn push_audio(
        &self,
        session_id: String,
        pcm: Vec<u8>,
    ) -> Result<Vec<SttEvent>, String> {
        let runtime = Arc::clone(&self.runtime);
        tokio::task::spawn_blocking(move || {
            let mut runtime = runtime.lock().unwrap_or_else(|error| error.into_inner());
            let SttRuntime { engine, session } = &mut *runtime;
            let engine = engine
                .as_ref()
                .ok_or_else(|| "STT engine is not loaded".to_string())?;
            let session = session
                .as_mut()
                .ok_or_else(|| "No STT session is active".to_string())?;
            if session.id() != session_id {
                return Err("STT session ID does not match the active session".into());
            }
            session.push_pcm16(engine, &pcm)
        })
        .await
        .map_err(|error| format!("STT audio task panicked: {error}"))?
    }

    pub async fn stop(&self, session_id: String) -> Result<Vec<SttEvent>, String> {
        let runtime = Arc::clone(&self.runtime);
        let listening = Arc::clone(&self.listening);
        tokio::task::spawn_blocking(move || {
            let mut runtime = runtime.lock().unwrap_or_else(|error| error.into_inner());
            if runtime.session.as_ref().map(SttSession::id) != Some(session_id.as_str()) {
                return Err("STT session ID does not match the active session".into());
            }
            let session = runtime.session.take().expect("active STT session");
            let result = session.finish(
                runtime
                    .engine
                    .as_ref()
                    .expect("STT session requires an engine"),
            );
            listening.store(false, Ordering::Release);
            result
        })
        .await
        .map_err(|error| format!("STT stop task panicked: {error}"))?
    }
}

pub fn emit_stt_events(
    sink: &dyn EventSink,
    session_id: &str,
    events: Vec<SttEvent>,
) -> Result<(), String> {
    for event in events {
        let (name, payload) = match event {
            SttEvent::Level { rms, peak } => (
                "stt-level",
                json!({ "sessionId": session_id, "rms": rms, "peak": peak }),
            ),
            SttEvent::SpeechStarted => (
                "stt-state",
                json!({ "sessionId": session_id, "state": "speech" }),
            ),
            SttEvent::Partial { sequence, text } => (
                "stt-partial",
                json!({ "sessionId": session_id, "sequence": sequence, "text": text }),
            ),
            SttEvent::Final {
                sequence,
                raw_text,
                text,
                repair_status,
            } => (
                "stt-final",
                json!({
                    "sessionId": session_id,
                    "sequence": sequence,
                    "rawText": raw_text,
                    "text": text,
                    "repairStatus": repair_status,
                }),
            ),
        };
        sink.emit(name, payload)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_models_make_status_explicit() {
        let service = SttService::with_config(
            &ResourceLocator::new(None, "/missing"),
            SttConfig::default(),
        );
        let status = service.status();
        assert!(!status.available);
        assert_eq!(status.capture_backend, "renderer");
        assert_eq!(status.language_mode, "auto");
        assert_eq!(status.supported_languages, ["tr", "en"]);
        assert!(status.reason.unwrap().contains("STT model is incomplete"));
    }

    #[test]
    fn system_control_does_not_change_manual_session_capability() {
        let service = SttService::disabled();
        assert!(!service.status().system_enabled);
        let status = service.set_system_enabled(true);
        assert!(status.system_enabled);
        assert!(!status.available);
        assert!(!status.listening);
    }
}
