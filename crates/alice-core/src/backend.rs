use crate::{EventSink, ResourceLocator};
use serde_json::Value;
use std::sync::Arc;

/// Host-provided facilities needed by backend operations.
///
/// The Tauri adapter supplies a Tauri event sink during migration. The Electron
/// sidecar will supply a framed-protocol event sink without changing business
/// logic.
pub struct BackendFoundation {
    resources: ResourceLocator,
    events: Arc<dyn EventSink>,
}

impl BackendFoundation {
    pub fn new(resources: ResourceLocator, events: Arc<dyn EventSink>) -> Self {
        Self { resources, events }
    }

    pub fn resources(&self) -> &ResourceLocator {
        &self.resources
    }

    pub fn events(&self) -> &dyn EventSink {
        self.events.as_ref()
    }

    pub fn emit(&self, name: &str, payload: Value) -> Result<(), String> {
        self.events.emit(name, payload)
    }
}
