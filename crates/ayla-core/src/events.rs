use serde_json::Value;

/// Runtime-neutral event output used by ingestion, search, OCR, and graph work.
pub trait EventSink: Send + Sync {
    fn emit(&self, name: &str, payload: Value) -> Result<(), String>;
}

/// Runtime-neutral per-request stream output, used for chat deltas and other
/// operations whose response arrives incrementally.
pub trait StreamSink: Send + Sync {
    fn send(&self, payload: Value) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _name: &str, _payload: Value) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NoopStreamSink;

impl StreamSink for NoopStreamSink {
    fn send(&self, _payload: Value) -> Result<(), String> {
        Ok(())
    }
}
