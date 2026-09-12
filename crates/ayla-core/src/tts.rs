//! Application-level TTS orchestration.
//!
//! Model execution lives in the internal `ayla-tts` crate. This facade owns
//! Ayla resource discovery, lazy loading, configuration, and cancellation.

use crate::config::{TtsConfig, TtsEnginePreference};
use crate::ResourceLocator;
use ayla_tts::{
    EngineKind, ModelConfig, PiperModel, SupertonicModel, SynthesisOptions, TtsEngine, TtsLanguage,
    SUPERTONIC_LANGUAGES,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    pub available: bool,
    pub loaded: bool,
    pub engine: Option<String>,
    pub voice: Option<String>,
    pub language: Option<String>,
    pub supported_languages: Vec<String>,
    pub provider: Option<String>,
    pub model_directory: Option<String>,
    pub reason: Option<String>,
}

impl TtsStatus {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            loaded: false,
            engine: None,
            voice: None,
            language: None,
            supported_languages: Vec::new(),
            provider: None,
            model_directory: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug)]
struct SelectedModel {
    config: ModelConfig,
    status: TtsStatus,
}

pub struct TtsService {
    engine: Arc<Mutex<Option<TtsEngine>>>,
    selected_model: Option<ModelConfig>,
    options: SynthesisOptions,
    default_language: TtsLanguage,
    threads: i32,
    cancellation_epoch: Arc<AtomicU64>,
    loaded: Arc<AtomicBool>,
    status: TtsStatus,
}

impl TtsService {
    pub fn new(resources: &ResourceLocator) -> Self {
        Self::with_config(resources, TtsConfig::default())
    }

    pub fn with_config(resources: &ResourceLocator, config: TtsConfig) -> Self {
        let selected = select_model(resources, &config);
        let (selected_model, status, default_language) = match selected {
            Ok(selected) => {
                let effective_language = selected
                    .status
                    .language
                    .as_deref()
                    .and_then(|value| TtsLanguage::parse(value).ok())
                    .unwrap_or(config.language);
                (Some(selected.config), selected.status, effective_language)
            }
            Err(reason) => {
                log::warn!("TTS unavailable: {reason}");
                (None, TtsStatus::unavailable(reason), config.language)
            }
        };
        Self {
            engine: Arc::new(Mutex::new(None)),
            selected_model,
            options: SynthesisOptions {
                speed: config.speed,
                num_steps: config.steps,
            },
            default_language,
            threads: config.threads,
            cancellation_epoch: Arc::new(AtomicU64::new(0)),
            loaded: Arc::new(AtomicBool::new(false)),
            status,
        }
    }

    pub fn disabled() -> Self {
        Self {
            engine: Arc::new(Mutex::new(None)),
            selected_model: None,
            options: SynthesisOptions::default(),
            default_language: TtsLanguage::Turkish,
            threads: 1,
            cancellation_epoch: Arc::new(AtomicU64::new(0)),
            loaded: Arc::new(AtomicBool::new(false)),
            status: TtsStatus::unavailable("TTS disabled in offline backend mode"),
        }
    }

    pub fn status(&self) -> TtsStatus {
        let mut status = self.status.clone();
        status.loaded = self.loaded.load(Ordering::Acquire);
        status
    }

    /// Synthesize text off the async runtime. The native progress callback
    /// observes the cancellation epoch and terminates superseded work.
    pub async fn generate(&self, text: String) -> Result<Vec<u8>, String> {
        self.generate_with_language(text, None).await
    }

    pub async fn generate_with_language(
        &self,
        text: String,
        language: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let language = language
            .map(TtsLanguage::parse)
            .transpose()?
            .unwrap_or(self.default_language);
        let model = self.selected_model.clone().ok_or_else(|| {
            self.status
                .reason
                .clone()
                .unwrap_or_else(|| "TTS unavailable".into())
        })?;
        let engine = Arc::clone(&self.engine);
        let options = self.options.clone();
        let threads = self.threads;
        let cancellation_epoch = Arc::clone(&self.cancellation_epoch);
        let request_epoch = cancellation_epoch.load(Ordering::Acquire);
        let loaded = Arc::clone(&self.loaded);
        tokio::task::spawn_blocking(move || {
            let mut guard = engine.lock().unwrap_or_else(|error| error.into_inner());
            if cancellation_epoch.load(Ordering::Acquire) != request_epoch {
                return Err("TTS generation cancelled".into());
            }
            if guard.is_none() {
                let created = TtsEngine::load(model, threads)?;
                log::info!(
                    "TTS loaded engine={} voice={} sample_rate={}",
                    created.kind().as_str(),
                    created.voice(),
                    created.sample_rate()
                );
                *guard = Some(created);
                loaded.store(true, Ordering::Release);
            }
            guard.as_ref().expect("TTS engine initialized").generate(
                &text,
                language,
                &options,
                cancellation_epoch,
                request_epoch,
            )
        })
        .await
        .map_err(|error| format!("TTS task panicked: {error}"))?
    }

    pub fn stop(&self) -> Result<(), String> {
        self.cancellation_epoch.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

fn select_model(resources: &ResourceLocator, config: &TtsConfig) -> Result<SelectedModel, String> {
    let supertonic_directory = config.model_dir.clone().unwrap_or_else(|| {
        resources.resolve_resource("tts/supertonic3-int8", "tts/supertonic3-int8")
    });
    let supertonic = SupertonicModel::new(&supertonic_directory);

    let piper_directory = config.model_dir.clone().unwrap_or_else(|| {
        resources.resolve_resource("tts/tts_piper_amy_low", "tts/tts_piper_amy_low")
    });
    let piper = PiperModel {
        model: piper_directory.join("en_US-amy-low.onnx"),
        tokens: piper_directory.join("tokens.txt"),
        data_dir: piper_directory.join("espeak-ng-data"),
    };

    match config.engine {
        TtsEnginePreference::Supertonic => select_supertonic(supertonic, config.language),
        TtsEnginePreference::Piper => select_piper(piper, piper_directory),
        TtsEnginePreference::Auto if supertonic.is_complete() => {
            select_supertonic(supertonic, config.language)
        }
        TtsEnginePreference::Auto if piper.is_complete() => select_piper(piper, piper_directory),
        TtsEnginePreference::Auto => Err(format!(
            "No complete TTS model found; checked Supertonic at {} and Piper at {}",
            supertonic_directory.display(),
            piper_directory.display()
        )),
    }
}

fn select_supertonic(
    model: SupertonicModel,
    language: TtsLanguage,
) -> Result<SelectedModel, String> {
    let missing = model.missing_files();
    if !missing.is_empty() {
        return Err(format!(
            "Supertonic model is incomplete at {} (missing: {})",
            model.directory.display(),
            missing.join(", ")
        ));
    }
    let directory = model.directory.to_string_lossy().into_owned();
    Ok(SelectedModel {
        config: ModelConfig::Supertonic(model),
        status: TtsStatus {
            available: true,
            loaded: false,
            engine: Some(EngineKind::Supertonic.as_str().into()),
            voice: Some("F1".into()),
            language: Some(language.code().into()),
            supported_languages: SUPERTONIC_LANGUAGES
                .iter()
                .map(|value| value.to_string())
                .collect(),
            provider: Some("cpu".into()),
            model_directory: Some(directory),
            reason: None,
        },
    })
}

fn select_piper(model: PiperModel, directory: PathBuf) -> Result<SelectedModel, String> {
    if !model.is_complete() {
        return Err(format!(
            "Piper model is incomplete at {}",
            directory.display()
        ));
    }
    Ok(SelectedModel {
        config: ModelConfig::Piper(model),
        status: TtsStatus {
            available: true,
            loaded: false,
            engine: Some(EngineKind::Piper.as_str().into()),
            voice: Some("amy-low".into()),
            language: Some("en".into()),
            supported_languages: vec!["en".into()],
            provider: Some("cpu".into()),
            model_directory: Some(directory.to_string_lossy().into_owned()),
            reason: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_models_are_reported_without_loading_native_runtime() {
        let resources = ResourceLocator::new(None, "/definitely/missing");
        let service = TtsService::with_config(&resources, TtsConfig::default());
        let status = service.status();
        assert!(!status.available);
        assert!(!status.loaded);
        assert!(status.reason.unwrap().contains("No complete TTS model"));
    }

    #[test]
    fn stop_advances_the_generation_epoch_without_an_engine_lock() {
        let service = TtsService::disabled();
        assert_eq!(service.cancellation_epoch.load(Ordering::Acquire), 0);
        service.stop().unwrap();
        assert_eq!(service.cancellation_epoch.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn request_language_is_validated_before_model_generation() {
        let service = TtsService::disabled();
        let error = service
            .generate_with_language("Hallo".into(), Some("de"))
            .await
            .unwrap_err();
        assert!(error.contains("must be 'tr'"));
    }

    #[test]
    fn piper_selection_reports_english_as_its_only_language() {
        let directory =
            std::env::temp_dir().join(format!("ayla-piper-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(directory.join("espeak-ng-data")).unwrap();
        std::fs::write(directory.join("model.onnx"), b"fixture").unwrap();
        std::fs::write(directory.join("tokens.txt"), b"fixture").unwrap();
        let model = PiperModel {
            model: directory.join("model.onnx"),
            tokens: directory.join("tokens.txt"),
            data_dir: directory.join("espeak-ng-data"),
        };
        let selected = select_piper(model, directory.clone()).unwrap();
        assert_eq!(selected.status.language.as_deref(), Some("en"));
        assert_eq!(selected.status.supported_languages, vec!["en"]);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
