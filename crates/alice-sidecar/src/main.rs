mod dispatcher;
mod protocol;
mod sinks;

use alice_core::commands::BACKEND_METHODS;
use alice_core::{BackendError, BackendState, EventSink, MemoryConfig, ResourceLocator};
use protocol::{read_frame, write_frame, IncomingMessage, OutgoingMessage, PROTOCOL_VERSION};
use sinks::{ProtocolEventSink, ProtocolStreamSink};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};

const MAX_OUTSTANDING_REQUESTS: usize = 64;

#[tokio::main]
async fn main() {
    if std::env::args().any(|argument| argument == "--protocol-version") {
        println!("{PROTOCOL_VERSION}");
        return;
    }
    if let Err(error) = serve().await {
        eprintln!("alice-sidecar: {error}");
        std::process::exit(1);
    }
}

async fn serve() -> Result<(), protocol::ProtocolError> {
    let mut input = tokio::io::stdin();
    let (output_sender, mut output_receiver) = mpsc::unbounded_channel::<OutgoingMessage>();
    let writer = tokio::spawn(async move {
        let mut output = tokio::io::stdout();
        while let Some(message) = output_receiver.recv().await {
            let _protocol_lock = alice_core::protocol_io_guard().lock().await;
            write_frame(&mut output, &message).await?;
        }
        Ok::<(), protocol::ProtocolError>(())
    });

    let resources = ResourceLocator::new(resource_directory(), development_root());
    let global_events: Arc<dyn EventSink> =
        Arc::new(ProtocolEventSink::global(output_sender.clone()));
    let state = Arc::new(if offline_mode() {
        BackendState::offline(
            MemoryConfig::default(),
            resources,
            Arc::clone(&global_events),
        )
    } else {
        BackendState::initialize(
            MemoryConfig::default(),
            resources,
            Arc::clone(&global_events),
        )
        .await
    });
    send_output(
        &output_sender,
        OutgoingMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            backend_version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
            methods: BACKEND_METHODS
                .iter()
                .map(|method| (*method).to_string())
                .collect(),
            capabilities: vec![
                "messagepack".into(),
                "binary".into(),
                "events".into(),
                "streaming".into(),
                "cancellation".into(),
            ],
        },
    )?;

    let request_slots = Arc::new(Semaphore::new(MAX_OUTSTANDING_REQUESTS));
    let mut requests = HashMap::<String, tokio::task::JoinHandle<()>>::new();
    while let Some(message) = read_frame::<_, IncomingMessage>(&mut input).await? {
        requests.retain(|_, task| !task.is_finished());
        match message {
            IncomingMessage::Ping { id } => {
                send_output(&output_sender, OutgoingMessage::Pong { id })?;
            }
            IncomingMessage::CancelRequest { request_id } => {
                if let Some(task) = requests.remove(&request_id) {
                    task.abort();
                }
            }
            IncomingMessage::Shutdown => break,
            IncomingMessage::Request { id, method, params } => {
                let Ok(permit) = request_slots.clone().try_acquire_owned() else {
                    send_output(
                        &output_sender,
                        OutgoingMessage::Response {
                            id,
                            result: None,
                            error: Some(
                                BackendError::new(
                                    "too_many_requests",
                                    "Too many outstanding backend requests",
                                )
                                .retryable(true),
                            ),
                        },
                    )?;
                    continue;
                };
                if !BACKEND_METHODS.contains(&method.as_str()) {
                    send_output(
                        &output_sender,
                        OutgoingMessage::Response {
                            id,
                            result: None,
                            error: Some(BackendError::new(
                                "method_not_allowed",
                                format!("Unknown backend command '{method}'"),
                            )),
                        },
                    )?;
                    continue;
                }

                let state = Arc::clone(&state);
                let sender = output_sender.clone();
                let request_id = id.clone();
                let task = tokio::spawn(async move {
                    let _permit = permit;
                    let events = ProtocolEventSink::for_request(sender.clone(), id.clone());
                    let stream = ProtocolStreamSink::new(sender.clone(), id.clone());
                    let response =
                        dispatcher::dispatch(&state, &method, params, &events, &stream).await;
                    let message = match response {
                        Ok(result) => OutgoingMessage::Response {
                            id,
                            result: Some(result),
                            error: None,
                        },
                        Err(error) => OutgoingMessage::Response {
                            id,
                            result: None,
                            error: Some(error),
                        },
                    };
                    let _ = sender.send(message);
                });
                requests.insert(request_id, task);
            }
        }
    }

    let tasks = requests.into_values().collect::<Vec<_>>();
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }
    drop(state);
    drop(global_events);
    drop(output_sender);
    match writer.await {
        Ok(result) => result?,
        Err(error) => {
            return Err(protocol::ProtocolError::Io(std::io::Error::other(format!(
                "output task failed: {error}"
            ))))
        }
    }
    Ok(())
}

fn send_output(
    sender: &mpsc::UnboundedSender<OutgoingMessage>,
    message: OutgoingMessage,
) -> Result<(), protocol::ProtocolError> {
    sender.send(message).map_err(|_| {
        protocol::ProtocolError::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "sidecar output channel closed",
        ))
    })
}

fn resource_directory() -> Option<PathBuf> {
    std::env::var_os("ALICE_RESOURCE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn development_root() -> PathBuf {
    std::env::var_os("ALICE_DEVELOPMENT_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri"))
}

fn offline_mode() -> bool {
    std::env::args().any(|argument| argument == "--offline")
}
