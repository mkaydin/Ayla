use ayla_core::{EventSink, StreamSink};
use serde_json::Value;
use tauri::Emitter;

/// Temporary adapter that forwards host-neutral backend events through Tauri.
pub struct TauriEventSink {
    app: tauri::AppHandle,
}

impl TauriEventSink {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl EventSink for TauriEventSink {
    fn emit(&self, name: &str, payload: Value) -> Result<(), String> {
        self.app
            .emit(name, payload)
            .map_err(|error| error.to_string())
    }
}

/// Per-request adapter used by streaming commands. The future sidecar will
/// implement the same trait with framed protocol messages.
pub struct TauriChannelStreamSink {
    channel: tauri::ipc::Channel<Value>,
}

impl TauriChannelStreamSink {
    pub fn new(channel: tauri::ipc::Channel<Value>) -> Self {
        Self { channel }
    }
}

impl StreamSink for TauriChannelStreamSink {
    fn send(&self, payload: Value) -> Result<(), String> {
        self.channel
            .send(payload)
            .map_err(|error| error.to_string())
    }
}
