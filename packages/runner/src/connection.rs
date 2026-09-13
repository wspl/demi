//! Bounded runner WebSocket transport with one explicitly joined owner.

use std::{io, time::Duration};

use demi_runner_protocol::{Inbound, Outbound};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use tokio_util::sync::CancellationToken;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const QUEUE_MESSAGES: usize = 8;

pub struct Connection {
    pub output: mpsc::Sender<Outbound>,
    pub control: mpsc::Sender<Outbound>,
    pub input: mpsc::Receiver<Inbound>,
    cancel: CancellationToken,
    owner: Option<JoinHandle<io::Result<()>>>,
}

pub fn socket_url(backend: &str) -> io::Result<reqwest::Url> {
    let mut url = crate::state::backend_url(backend)?;
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        _ => "wss",
    };
    url.set_scheme(scheme)
        .map_err(|_| io::Error::other("invalid WebSocket scheme"))?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/api/runner");
    }
    Ok(url)
}

impl Connection {
    pub async fn connect(backend: &str, cancel: CancellationToken) -> io::Result<Self> {
        let url = socket_url(backend)?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let (socket, _) = tokio::select! {
            _ = cancel.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted, "connection cancelled")),
            result = tokio::time::timeout(HANDSHAKE_TIMEOUT, tokio_tungstenite::connect_async_with_config(url.as_str(), Some(config), true)) => {
                result.map_err(io::Error::other)?.map_err(io::Error::other)?
            }
        };
        Ok(Self::from_socket(socket, cancel))
    }

    pub fn from_socket<S>(socket: WebSocketStream<S>, cancel: CancellationToken) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let cancel = cancel.child_token();
        let (output, mut outgoing) = mpsc::channel::<Outbound>(QUEUE_MESSAGES);
        let (control, mut controls) = mpsc::channel::<Outbound>(128);
        let (incoming, input) = mpsc::channel(QUEUE_MESSAGES);
        let stopped = cancel.clone();
        let owner = tokio::spawn(async move {
            let (mut writer, mut reader) = socket.split();
            let receive = async {
                while let Some(message) = reader.next().await {
                    match message.map_err(io::Error::other)? {
                        Message::Binary(bytes) => {
                            let message =
                                demi_runner_protocol::decode(&bytes).map_err(io::Error::other)?;
                            // Overload closes the connection instead of blocking its
                            // reader behind application work or buffering without limit.
                            incoming.try_send(message).map_err(|error| {
                                io::Error::other(format!(
                                    "runner inbound queue unavailable: {error}"
                                ))
                            })?;
                        }
                        Message::Close(_) => return Ok(()),
                        Message::Ping(_) | Message::Pong(_) => {}
                        _ => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "runner requires binary WebSocket messages",
                            ));
                        }
                    }
                }
                Ok::<_, io::Error>(())
            };
            let send = async {
                loop {
                    let message = tokio::select! {
                        biased;
                        Some(message) = controls.recv() => message,
                        Some(message) = outgoing.recv() => message,
                        else => break,
                    };
                    let bytes = message.into_bytes();
                    if bytes.len() > MAX_MESSAGE_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "runner outbound message exceeds 16 MiB",
                        ));
                    }
                    tokio::time::timeout(WRITE_TIMEOUT, writer.send(Message::Binary(bytes.into())))
                        .await
                        .map_err(io::Error::other)?
                        .map_err(io::Error::other)?;
                }
                Ok::<_, io::Error>(())
            };
            // Both halves are owned by this task. Returning drops both futures
            // and the socket even when a write is stalled by a disconnected peer.
            tokio::select! {
                _ = stopped.cancelled() => Ok(()),
                result = receive => result,
                result = send => result,
            }
        });
        Self {
            output,
            control,
            input,
            cancel,
            owner: Some(owner),
        }
    }

    pub async fn close(mut self) -> io::Result<()> {
        self.cancel.cancel();
        self.owner
            .take()
            .expect("connection owner exists until close")
            .await
            .map_err(io::Error::other)?
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Cancellation wakes the owner on every early return; explicit close joins it.
        self.cancel.cancel();
    }
}
