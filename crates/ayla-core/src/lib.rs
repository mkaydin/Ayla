//! Host-independent backend foundations shared by the temporary Tauri host and
//! the production Electron Rust sidecar.

pub mod api;
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
pub mod tools;
pub mod tts;

pub use api::BackendApi;
pub use backend::BackendFoundation;
pub use config::MemoryConfig;
pub use error::BackendError;
pub use events::{EventSink, NoopEventSink, NoopStreamSink, StreamSink};
pub use memory::MemoryManager;
pub use processes::ProcessRegistry;
pub use protocol_io::protocol_io_guard;
pub use resources::ResourceLocator;
pub use state::BackendState;
pub use tts::TtsService;
