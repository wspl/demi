use bytes::Bytes;
use http::{Method, Request};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::protocol::{
    INFO_PATH, INVOKE_PATH, Invocation, MAX_METADATA_BYTES, ProtocolError, Record, RecordDecoder,
    SHUTDOWN_PATH, ServiceInfo, VERSION,
};
use crate::{ServiceError, stream::send_bytes};

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
            .initial_window_size(64 * 1024)
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
        let operations: std::collections::HashSet<_> = info.operations.iter().collect();
        if info.protocol_version != VERSION
            || info.operations.is_empty()
            || info.operations.iter().any(String::is_empty)
            || operations.len() != info.operations.len()
        {
            return Err(ServiceError::InvalidCatalog);
        }
        Ok(info)
    }

    pub async fn invoke(
        &self,
        invocation: &Invocation,
    ) -> Result<(CommandInput, CommandOutput), ServiceError> {
        let metadata = invocation.encode()?;
        let mut sender = self.sender.clone().ready().await?;
        let (response, mut input) =
            sender.send_request(request(Method::POST, INVOKE_PATH), false)?;
        send_bytes(&mut input, metadata).await?;
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
        send_bytes(&mut self.stream, bytes).await
    }
    pub fn end(&mut self) -> Result<(), ServiceError> {
        self.stream.send_data(Bytes::new(), true)?;
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.stream.send_reset(h2::Reason::CANCEL);
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
