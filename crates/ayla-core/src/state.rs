use crate::{
    BackendApi, BackendFoundation, EventSink, MemoryConfig, MemoryManager, ProcessRegistry,
    ResourceLocator, TtsService,
};
use std::sync::Arc;

/// Complete host-neutral backend ownership aggregate.
///
/// Tauri keeps its existing separately managed state during parity testing.
/// The Electron sidecar will own one of these and dispatch every request
/// through the contained services.
pub struct BackendState {
    foundation: BackendFoundation,
    memory: MemoryManager,
    processes: ProcessRegistry,
    tts: TtsService,
}

impl BackendState {
    pub async fn initialize(
        config: MemoryConfig,
        resources: ResourceLocator,
        events: Arc<dyn EventSink>,
    ) -> Self {
        let tts = TtsService::new(&resources);
        let memory = MemoryManager::new(config).await;
        Self {
            foundation: BackendFoundation::new(resources, events),
            memory,
            processes: ProcessRegistry::new(),
            tts,
        }
    }

    pub fn offline(
        config: MemoryConfig,
        resources: ResourceLocator,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            foundation: BackendFoundation::new(resources, events),
            memory: MemoryManager::offline(config),
            processes: ProcessRegistry::new(),
            tts: TtsService::disabled(),
        }
    }

    pub fn api(&self) -> BackendApi<'_> {
        BackendApi::new(&self.memory)
    }

    pub fn foundation(&self) -> &BackendFoundation {
        &self.foundation
    }

    pub fn memory(&self) -> &MemoryManager {
        &self.memory
    }

    pub fn processes(&self) -> &ProcessRegistry {
        &self.processes
    }

    pub fn tts(&self) -> &TtsService {
        &self.tts
    }
}
