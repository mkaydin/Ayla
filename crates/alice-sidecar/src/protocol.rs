use alice_core::BackendError;
use rmpv::Value;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncomingMessage {
    Request {
        id: String,
        method: String,
        #[serde(default = "empty_map")]
        params: Value,
    },
    Ping {
        id: String,
    },
    CancelRequest {
        request_id: String,
    },
    Shutdown,
}

fn empty_map() -> Value {
    Value::Map(Vec::new())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutgoingMessage {
    Hello {
        protocol_version: u32,
        backend_version: String,
        pid: u32,
        methods: Vec<String>,
        capabilities: Vec<String>,
    },
    Response {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<BackendError>,
    },
    Event {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        operation_id: Option<String>,
        payload: Value,
    },
    Stream {
        request_id: String,
        event: Value,
    },
    Pong {
        id: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame length {actual} exceeds maximum {maximum}")]
    FrameTooLarge { actual: usize, maximum: usize },
    #[error("protocol I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("MessagePack encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("MessagePack decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
}

pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>, ProtocolError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut header = [0_u8; 4];
    if reader.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..]).await?;
    let frame_size = u32::from_be_bytes(header) as usize;
    if frame_size > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge {
            actual: frame_size,
            maximum: MAX_FRAME_SIZE,
        });
    }
    let mut payload = vec![0_u8; frame_size];
    reader.read_exact(&mut payload).await?;
    Ok(Some(rmp_serde::from_slice(&payload)?))
}

pub async fn write_frame<W, T>(writer: &mut W, message: &T) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = rmp_serde::to_vec_named(message)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge {
            actual: payload.len(),
            maximum: MAX_FRAME_SIZE,
        });
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn fragmented_frame_round_trips() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let message = IncomingMessage::Request {
            id: "request-1".into(),
            method: "check_db_ready".into(),
            params: Value::Map(Vec::new()),
        };
        let payload = rmp_serde::to_vec_named(&message).unwrap();
        let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
        frame.extend(payload);
        tokio::spawn(async move {
            for byte in frame {
                client.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
        });

        let decoded: IncomingMessage = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(decoded, message);
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_before_payload_allocation() {
        let (mut client, mut server) = tokio::io::duplex(16);
        client
            .write_all(&((MAX_FRAME_SIZE as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        let error = read_frame::<_, IncomingMessage>(&mut server)
            .await
            .unwrap_err();
        assert!(matches!(error, ProtocolError::FrameTooLarge { .. }));
    }

    #[tokio::test]
    async fn binary_response_remains_messagepack_binary() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let message = OutgoingMessage::Response {
            id: "tts-1".into(),
            result: Some(Value::Binary(vec![0, 1, 2, 255])),
            error: None,
        };
        write_frame(&mut client, &message).await.unwrap();
        let decoded: OutgoingMessage = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(decoded, message);
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (client, mut server) = tokio::io::duplex(16);
        drop(client);
        assert!(read_frame::<_, IncomingMessage>(&mut server)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn request_cancellation_uses_a_distinct_control_message() {
        let message = IncomingMessage::CancelRequest {
            request_id: "request-1".into(),
        };
        let payload = rmp_serde::to_vec_named(&message).unwrap();
        let decoded: IncomingMessage = rmp_serde::from_slice(&payload).unwrap();
        assert_eq!(decoded, message);
    }
}
