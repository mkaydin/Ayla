//! the production Electron Rust sidecar.

pub mod api;
pub mod artifacts;
pub mod backend;
pub mod commands;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod memory;
pub mod processes;
pub mod protocol_io;
pub mod research;
pub mod resources;
pub mod state;
pub mod stt;
pub mod tools;
pub mod tts;

pub use api::BackendApi;
pub use artifacts::{ArtifactContent, ArtifactRef, CodeAnalysisArtifacts, OcrArtifacts};
pub use ayla_tts::TtsLanguage;
pub use backend::BackendFoundation;
pub use config::{MemoryConfig, SttConfig, TtsConfig, TtsEnginePreference};
pub use error::BackendError;
pub use events::{EventSink, NoopEventSink, NoopStreamSink, StreamSink};
pub use memory::MemoryManager;
pub use processes::ProcessRegistry;
pub use protocol_io::protocol_io_guard;
pub use resources::ResourceLocator;
pub use state::{BackendState, RuntimeCapabilities};
pub use stt::{emit_stt_events, SttService, SttStatus};
pub use tts::{TtsService, TtsStatus};

pub fn recover_research_jobs(data_root: &std::path::Path) -> Result<usize, String> {
    ayla_research::job::ResearchJob::recover(&data_root.join("research"))
        .map_err(|error| error.to_string())
}
