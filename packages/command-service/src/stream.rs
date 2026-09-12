use bytes::{Buf, Bytes, BytesMut};
use futures_util::future::poll_fn;
use h2::{RecvStream, SendStream};
use thiserror::Error;

use crate::protocol::{Invocation, MAX_METADATA_BYTES, ProtocolError};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Http2(#[from] h2::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("command cancelled")]
    Cancelled,
    #[error("command handler failed: {0}")]
    Handler(String),
    #[error("invalid service operation catalog")]
    InvalidCatalog,
    #[error("handler exceeded cancellation deadline; retire the service process")]
    CancellationDeadline,
    #[error("HTTP/2 service handshake timed out")]
    HandshakeTimeout,
    #[error("service rejected HTTP request with status {0}")]
    Rejected(u16),
}

pub struct Input {
    stream: RecvStream,
    pending: Bytes,
    delivered: usize,
}

impl Input {
    pub(crate) fn new(stream: RecvStream) -> Self {
        Self {
            stream,
            pending: Bytes::new(),
            delivered: 0,
        }
    }

    /// Returns capacity for the previous chunk only when the consumer asks again.
    pub async fn next(&mut self) -> Result<Option<Bytes>, ServiceError> {
        self.stream
            .flow_control()
            .release_capacity(self.delivered)?;
        self.delivered = 0;
        let chunk = if self.pending.is_empty() {
            self.stream.data().await.transpose()?
        } else {
            Some(std::mem::take(&mut self.pending))
        };
        if let Some(bytes) = &chunk {
            self.delivered = bytes.len();
        }
        Ok(chunk)
    }

    pub(crate) async fn invocation(&mut self) -> Result<Invocation, ServiceError> {
        let prefix = self.read_exact(4).await?;
        let length = u32::from_be_bytes(prefix[..].try_into().unwrap()) as usize;
        if length > MAX_METADATA_BYTES {
            return Err(ProtocolError::TooLarge.into());
        }
        let json = self.read_exact(length).await?;
        let invocation: Invocation = serde_json::from_slice(&json)?;
        invocation.validate()?;
        Ok(invocation)
    }

    async fn read_exact(&mut self, length: usize) -> Result<Bytes, ServiceError> {
        let mut result = BytesMut::with_capacity(length);
        while result.len() < length {
            if self.pending.is_empty() {
                self.pending = self
                    .stream
                    .data()
                    .await
                    .ok_or(ProtocolError::Incomplete)??;
            }
            let count = (length - result.len()).min(self.pending.len());
            result.extend_from_slice(&self.pending[..count]);
            self.pending.advance(count);
            self.stream.flow_control().release_capacity(count)?;
        }
        Ok(result.freeze())
    }
}

pub(crate) async fn send_bytes(
    stream: &mut SendStream<Bytes>,
    mut bytes: Bytes,
) -> Result<(), ServiceError> {
    while !bytes.is_empty() {
        stream.reserve_capacity(bytes.len());
        let capacity = poll_fn(|cx| stream.poll_capacity(cx))
            .await
            .ok_or(ServiceError::Cancelled)??;
        if capacity == 0 {
            continue;
        }
        let count = capacity.min(bytes.len());
        stream.send_data(bytes.split_to(count), false)?;
    }
    Ok(())
}
