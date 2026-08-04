use crate::protocol::OutgoingMessage;
use alice_core::{EventSink, StreamSink};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub struct ProtocolEventSink {
    sender: UnboundedSender<OutgoingMessage>,
    operation_id: Option<String>,
}

impl ProtocolEventSink {
    pub fn global(sender: UnboundedSender<OutgoingMessage>) -> Self {
        Self {
            sender,
            operation_id: None,
        }
    }

    pub fn for_request(sender: UnboundedSender<OutgoingMessage>, request_id: String) -> Self {
        Self {
            sender,
            operation_id: Some(request_id),
        }
    }
}

impl EventSink for ProtocolEventSink {
    fn emit(&self, name: &str, payload: serde_json::Value) -> Result<(), String> {
        let payload = rmpv::ext::to_value(payload)
            .map_err(|error| format!("Event payload serialization: {error}"))?;
        self.sender
            .send(OutgoingMessage::Event {
                name: name.to_string(),
                operation_id: self.operation_id.clone(),
                payload,
            })
            .map_err(|_| "Sidecar output channel closed".to_string())
    }
}

#[derive(Clone)]
pub struct ProtocolStreamSink {
    sender: UnboundedSender<OutgoingMessage>,
    request_id: String,
}

impl ProtocolStreamSink {
    pub fn new(sender: UnboundedSender<OutgoingMessage>, request_id: String) -> Self {
        Self { sender, request_id }
    }
}

impl StreamSink for ProtocolStreamSink {
    fn send(&self, payload: serde_json::Value) -> Result<(), String> {
        let event = rmpv::ext::to_value(payload)
            .map_err(|error| format!("Stream payload serialization: {error}"))?;
        self.sender
            .send(OutgoingMessage::Stream {
                request_id: self.request_id.clone(),
                event,
            })
            .map_err(|_| "Sidecar output channel closed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alice_core::{EventSink, StreamSink};

    #[test]
    fn request_sinks_attach_correlation_ids() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let events = ProtocolEventSink::for_request(sender.clone(), "request-1".into());
        let stream = ProtocolStreamSink::new(sender, "request-1".into());
        events
            .emit("ingest-progress", serde_json::json!({"pct": 50}))
            .unwrap();
        stream
            .send(serde_json::json!({"type": "delta", "content": "hi"}))
            .unwrap();

        assert!(matches!(
            receiver.try_recv().unwrap(),
            OutgoingMessage::Event {
                operation_id: Some(id),
                ..
            } if id == "request-1"
        ));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            OutgoingMessage::Stream { request_id, .. } if request_id == "request-1"
        ));
    }
}
