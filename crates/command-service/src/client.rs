use bytes::Bytes;
use http::{Method, Request};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::protocol::{
    CONVERSATION_PATH, ConversationRequest, INFO_PATH, INVOKE_PATH, MAX_METADATA_BYTES,
    MAX_RECORD_BYTES, Metadata, ProtocolError, Record, RecordDecoder, SHUTDOWN_PATH, ServiceInfo,
};
use crate::{
    ServiceError,
    stream::{CONNECTION_WINDOW, send_bytes},
};

/// The owner continuously drives the returned connection and closes it on teardown.
#[derive(Clone)]
pub struct Client {
    sender: h2::client::SendRequest<Bytes>,
}

impl Client {
    pub async fn connect<T>(io: T) -> Result<(Self, h2::client::Connection<T, Bytes>), ServiceError>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        let (sender, connection) = h2::client::Builder::new()
            .max_header_list_size(16 * 1024)
            .initial_window_size(MAX_RECORD_BYTES as u32)
            .initial_connection_window_size(CONNECTION_WINDOW)
            .handshake(io)
            .await?;
        Ok((Self { sender }, connection))
    }

    pub async fn info(&self) -> Result<ServiceInfo, ServiceError> {
        let mut sender = self.sender.clone().ready().await?;
        let (response, _input) = sender.send_request(request(Method::GET, INFO_PATH), true)?;
        let response = response.await?;
        if !response.status().is_success() {
            return Err(ServiceError::Rejected(response.status().as_u16()));
        }
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(chunk) = body.data().await {
            let chunk = chunk?;
            if bytes.len() + chunk.len() > MAX_METADATA_BYTES {
                return Err(ProtocolError::TooLarge.into());
            }
            bytes.extend_from_slice(&chunk);
            body.flow_control().release_capacity(chunk.len())?;
        }
        let info: ServiceInfo = serde_json::from_slice(&bytes)?;
        info.validate()?;
        Ok(info)
    }

    pub async fn invoke(
        &self,
        invocation: &impl Metadata,
    ) -> Result<(CommandInput, CommandOutput), ServiceError> {
        self.invoke_at(INVOKE_PATH, invocation.encode()?).await
    }

    /// Conversation lifecycle has a finite metadata-only body, already ended on return.
    /// The returned input handle is retained only to cancel the response stream.
    pub async fn conversation(
        &self,
        request: &ConversationRequest,
    ) -> Result<(CommandInput, CommandOutput), ServiceError> {
        self.invoke_at(CONVERSATION_PATH, request.encode()?).await
    }

    async fn invoke_at(
        &self,
        path: &str,
        metadata: Bytes,
    ) -> Result<(CommandInput, CommandOutput), ServiceError> {
        let mut sender = self.sender.clone().ready().await?;
        let (response, mut input) = sender.send_request(request(Method::POST, path), false)?;
        if path == CONVERSATION_PATH {
            // The metadata is the whole request, so it leaves with its end:
            // nothing is left to send once the service can answer it
            // (`native-runtime.md` § Request body and input demand).
            input.send_data(metadata, true)?;
        } else {
            send_bytes(&mut input, metadata).await?;
        }
        let response = response.await?;
        if !response.status().is_success() {
            input.send_reset(h2::Reason::CANCEL);
            return Err(ServiceError::Rejected(response.status().as_u16()));
        }
        Ok((
            CommandInput { stream: input },
            CommandOutput {
                stream: response.into_body(),
                pending: Bytes::new(),
                decoder: Some(RecordDecoder::default()),
            },
        ))
    }

    pub async fn shutdown(&self) -> Result<(), ServiceError> {
        let mut sender = self.sender.clone().ready().await?;
        let (response, _input) = sender.send_request(request(Method::POST, SHUTDOWN_PATH), true)?;
        let response = response.await?;
        if !response.status().is_success() {
            return Err(ServiceError::Rejected(response.status().as_u16()));
        }
        Ok(())
    }
}

fn request(method: Method, path: &str) -> Request<()> {
    Request::builder()
        .method(method)
        .uri(format!("http://demi{path}"))
        .body(())
        .expect("constant service endpoint")
}

pub struct CommandInput {
    stream: h2::SendStream<Bytes>,
}

impl CommandInput {
    pub async fn write(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        let sent = send_bytes(&mut self.stream, crate::protocol::encode_input(bytes)?).await;
        self.unless_answered(sent)
    }
    pub fn end(&mut self) -> Result<(), ServiceError> {
        let sent = self.stream.send_data(Bytes::new(), true);
        self.unless_answered(sent.map_err(ServiceError::from))
    }
    pub fn cancel(&mut self) {
        self.stream.send_reset(h2::Reason::CANCEL);
    }

    /// A service that finished without reading all input resets the request
    /// with NO_ERROR once its response is complete; input sent after that,
    /// a chunk or its end, is dropped, not a failure (`native-runtime.md`
    /// § Request body and input demand).
    fn unless_answered(&mut self, sent: Result<(), ServiceError>) -> Result<(), ServiceError> {
        let Err(error) = sent else {
            return Ok(());
        };
        // h2 has no synchronous query for a reset the peer already sent;
        // polling once with a waker that never wakes reads it without waiting.
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match self.stream.poll_reset(&mut context) {
            std::task::Poll::Ready(Ok(h2::Reason::NO_ERROR)) => Ok(()),
            _ => Err(error),
        }
    }
}

pub struct CommandOutput {
    stream: h2::RecvStream,
    pending: Bytes,
    decoder: Option<RecordDecoder>,
}

impl CommandOutput {
    pub async fn next(&mut self) -> Result<Option<Record>, ServiceError> {
        let Some(decoder) = &mut self.decoder else {
            return Ok(None);
        };
        loop {
            if !self.pending.is_empty() {
                let before = self.pending.len();
                let record = decoder.decode(&mut self.pending)?;
                self.stream
                    .flow_control()
                    .release_capacity(before - self.pending.len())?;
                if record.is_some() {
                    return Ok(record);
                }
            }
            match self.stream.data().await {
                Some(bytes) => self.pending = bytes?,
                None => {
                    self.decoder
                        .take()
                        .expect("decoder exists until EOF")
                        .finish()?;
                    return Ok(None);
                }
            }
        }
    }
}
