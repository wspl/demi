//! The backend WebSocket (`runner.md` § Connection and identity): one owner
//! task reads messages into a queue of eight and writes the queued replies.
//! When the queue is full the reader waits, so the backend's writes wait too;
//! only a message that breaks the protocol closes the connection. A
//! connection the runner ends on purpose, as a drain or a stop does, ends
//! with a close frame.

use std::{io, time::Duration};

use super::wire::{self, Frame, Inbound};
use demi_runner_protocol::values::BackendUrl;
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};
use tokio_util::sync::CancellationToken;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long an ending the runner chose may take to send what is queued, its
/// close frame and to hear the backend's; a backend that takes longer sees
/// the connection end without them.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const QUEUE_MESSAGES: usize = 8;

pub struct Transport {
    /// Job and process output, behind the control replies.
    pub output: mpsc::Sender<Frame>,
    /// Replies and requests, written first.
    pub control: mpsc::Sender<Frame>,
    pub input: mpsc::Receiver<Inbound>,
    cancel: CancellationToken,
    owner: Option<JoinHandle<io::Result<()>>>,
}

/// The backend's runner socket: its `ws`/`wss` form, at `/api/runner` unless
/// the URL names another path.
pub fn socket_url(backend: &BackendUrl) -> io::Result<reqwest::Url> {
    let mut url = backend.url().clone();
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

impl Transport {
    pub async fn connect(backend: &BackendUrl, cancel: CancellationToken) -> io::Result<Self> {
        let url = socket_url(backend)?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(wire::MAX_MESSAGE_BYTES))
            .max_frame_size(Some(wire::MAX_MESSAGE_BYTES));
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
        let (output, mut outgoing) = mpsc::channel::<Frame>(QUEUE_MESSAGES);
        let (control, mut controls) = mpsc::channel::<Frame>(128);
        let (incoming, input) = mpsc::channel(QUEUE_MESSAGES);
        let stopped = cancel.clone();
        let owner = tokio::spawn(async move {
            let (mut writer, mut reader) = socket.split();
            let receive = async {
                while let Some(message) = reader.next().await {
                    match message.map_err(io::Error::other)? {
                        Message::Binary(bytes) => {
                            let message = wire::decode(&bytes).map_err(io::Error::other)?;
                            // A full queue stops the reading until the
                            // connection's owner takes a message.
                            if incoming.send(message).await.is_err() {
                                return Ok(());
                            }
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
                    write(&mut writer, message).await?;
                }
                Ok::<_, io::Error>(())
            };
            // Both halves are owned by this task. Returning drops both futures
            // and the socket even when a write is stalled by a disconnected peer.
            let ended = tokio::select! {
                _ = stopped.cancelled() => None,
                result = receive => Some(result),
                result = send => Some(result),
            };
            if let Some(result) = ended {
                return result;
            }
            // The runner ends this connection on purpose: what is queued
            // goes first, then a close frame, and the connection ends with
            // the backend's answer, so the backend sees the runner leave
            // rather than lose it (`runner.md` § Connection and identity).
            let close = async {
                while let Ok(message) = controls.try_recv().or_else(|_| outgoing.try_recv()) {
                    write(&mut writer, message).await?;
                }
                let frame = CloseFrame {
                    code: CloseCode::Away,
                    reason: "runner stopping".into(),
                };
                writer.send(Message::Close(Some(frame))).await.map_err(io::Error::other)?;
                while let Some(message) = reader.next().await {
                    if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                        break;
                    }
                }
                Ok::<_, io::Error>(())
            };
            // The connection is over either way: a backend that is gone or
            // slow only misses the orderly end.
            let _closed = tokio::time::timeout(CLOSE_TIMEOUT, close).await;
            Ok(())
        });
        Self {
            output,
            control,
            input,
            cancel,
            owner: Some(owner),
        }
    }

    /// A transport whose peer is the returned channels, for tests and
    /// in-process peers: the peer sends messages into the first and reads
    /// every frame, control or output, in the order sent from the second.
    pub fn channels() -> (Self, mpsc::Sender<Inbound>, mpsc::Receiver<Frame>) {
        let (output, frames) = mpsc::channel(128);
        let (inbound, input) = mpsc::channel(QUEUE_MESSAGES);
        (
            Self {
                control: output.clone(),
                output,
                input,
                cancel: CancellationToken::new(),
                owner: None,
            },
            inbound,
            frames,
        )
    }

    pub async fn close(mut self) -> io::Result<()> {
        self.cancel.cancel();
        match self.owner.take() {
            Some(owner) => owner.await.map_err(io::Error::other)?,
            None => Ok(()),
        }
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

/// Writes one queued frame. Replies over the limit already failed their
/// requests; anything else this large breaks the protocol.
async fn write<S>(writer: &mut S, message: Frame) -> io::Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let bytes = message.into_bytes();
    if bytes.len() > wire::MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("runner outbound message exceeds {} bytes", wire::MAX_MESSAGE_BYTES),
        ));
    }
    tokio::time::timeout(WRITE_TIMEOUT, writer.send(Message::Binary(bytes.into())))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)
}

impl Drop for Transport {
    fn drop(&mut self) {
        // Cancellation wakes the owner on every early return; explicit close joins it.
        self.cancel.cancel();
    }
}
