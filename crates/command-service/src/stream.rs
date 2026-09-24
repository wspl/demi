use bytes::{Buf, Bytes, BytesMut};
use futures_util::future::poll_fn;
use h2::{RecvStream, SendStream};
use thiserror::Error;

use crate::Output;
use crate::protocol::{MAX_METADATA_BYTES, MAX_RECORD_BYTES, ProtocolError};

/// The largest connection window HTTP/2 allows. Each stream keeps its own
/// record-sized window, so a stream's window always binds first and a slow
/// reader holds back only its own invocation.
pub(crate) const CONNECTION_WINDOW: u32 = (1 << 31) - 1;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Http2(#[from] h2::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("command cancelled")]
    Cancelled,
    /// The service has no such operation.
    #[error("unknown operation {0}")]
    UnknownOperation(String),
    /// The command failed; the cause says why, and is the whole message.
    #[error(transparent)]
    Failed(Box<dyn std::error::Error + Send + Sync>),
    /// The handler's task panicked or was aborted.
    #[error("command handler failed: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("conversation cleanup failed; retire the service process: {0}")]
    ConversationCleanup(String),
    #[error("handler exceeded cancellation deadline; retire the service process")]
    CancellationDeadline,
    #[error("HTTP/2 service handshake timed out")]
    HandshakeTimeout,
    #[error("service rejected HTTP request with status {0}")]
    Rejected(u16),
}

impl ServiceError {
    /// A command that failed for `cause`: an error of the handler's own, or
    /// a message.
    pub fn failed(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Failed(cause.into())
    }
}

/// Pull-driven input shared by local dispatch and HTTP/2 services.
pub struct Input(InputSource);

type LocalInput =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, ServiceError>> + Send>>;

enum InputSource {
    Http(HttpInput),
    /// `SyncWrapper` makes the boxed stream `Sync` by only ever lending it
    /// mutably, which `next` does.
    Local(sync_wrapper::SyncWrapper<LocalInput>),
}

impl Input {
    pub fn from_stream(
        stream: impl futures_util::Stream<Item = Result<Bytes, ServiceError>> + Send + 'static,
    ) -> Self {
        Self(InputSource::Local(sync_wrapper::SyncWrapper::new(Box::pin(stream))))
    }

    /// The input of an invocation whose metadata `input` has read.
    pub(crate) fn http(input: HttpInput) -> Self {
        Self(InputSource::Http(input))
    }

    pub async fn next(&mut self) -> Result<Option<Bytes>, ServiceError> {
        match &mut self.0 {
            InputSource::Http(input) => input.next().await,
            InputSource::Local(input) => poll_fn(|cx| input.get_mut().as_mut().poll_next(cx))
                .await
                .transpose(),
        }
    }
}

/// An invocation's HTTP/2 request body: its metadata, then its input.
pub(crate) struct HttpInput {
    stream: RecvStream,
    pending: Bytes,
    output: Option<Output>,
    ended: bool,
}

impl HttpInput {
    pub(crate) fn new(stream: RecvStream) -> Self {
        Self {
            stream,
            pending: Bytes::new(),
            output: None,
            ended: false,
        }
    }

    pub(crate) fn set_output(&mut self, output: Output) {
        self.output = Some(output);
    }

    /// Requests exactly one caller chunk. HTTP/2 capacity is transport
    /// backpressure, not permission to read from the caller's live stdin.
    pub async fn next(&mut self) -> Result<Option<Bytes>, ServiceError> {
        if self.ended {
            return Ok(None);
        }
        if self.pending.is_empty() && self.stream.is_end_stream() {
            self.ended = true;
            return Ok(None);
        }
        self.output
            .as_ref()
            .expect("input attached before handler invocation")
            .pull()
            .await?;
        while self.pending.is_empty() {
            match self.stream.data().await.transpose()? {
                Some(bytes) => self.pending = bytes,
                None => {
                    self.ended = true;
                    return Ok(None);
                }
            }
        }
        let prefix = self.read_exact(4).await?;
        let length = u32::from_be_bytes(prefix[..].try_into().unwrap()) as usize;
        if length > MAX_RECORD_BYTES {
            return Err(ProtocolError::TooLarge.into());
        }
        Ok(Some(self.read_exact(length).await?))
    }

    pub(crate) async fn metadata<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Result<T, ServiceError> {
        let prefix = self.read_exact(4).await?;
        let length = u32::from_be_bytes(prefix[..].try_into().unwrap()) as usize;
        if length > MAX_METADATA_BYTES {
            return Err(ProtocolError::TooLarge.into());
        }
        let json = self.read_exact(length).await?;
        Ok(serde_json::from_slice(&json)?)
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
