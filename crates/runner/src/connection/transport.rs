//! The backend WebSocket: one owner task reads messages into a queue and
//! writes the queued replies.
//!
//! - The queues of the backend's messages, of control and of output have no
//!   length limit, so the reader never waits for work that does not read, and
//!   nothing is cancelled or dropped for a count. Only a message that breaks
//!   the protocol closes the connection (the limits and why:
//!   `demi_runner_protocol::wire`).
//! - A message the runner cannot decode ends the connection with a close frame
//!   naming its type and the decoding error.
//! - A connection the runner ends on purpose, as a drain or a stop does, ends
//!   in order: the queued messages, a close frame, and up to `CLOSE_TIMEOUT`
//!   for the backend's. The backend sees the runner leave rather than lose it.
//! - The runner pings the backend and counts a connection it hears nothing on
//!   as lost, as one the network dropped without closing it is.
//! - The link's session outlives the connection ([`LinkState`]): the writer
//!   numbers each message where it writes it and keeps it until the backend
//!   confirms it, and after the hello is answered it writes again what the
//!   backend did not receive. The messages of calls wait in a backlog that
//!   outlives the transport, the rest in the transport's queues, which end
//!   with it.

use std::{io, sync::Arc, time::Duration};

use demi_command_protocol::host_target;
use demi_runner_process::backend::Backend;
use demi_runner_protocol::{
    release::{RELEASE_HEADER, RunnerUpdate, TARGET_HEADER, TOKEN_HEADER},
    values::DeviceToken,
    wire::{self, Frame, Inbound, Read},
};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Error, Message,
        client::IntoClientRequest,
        http::{HeaderValue, StatusCode},
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};
use tokio_util::sync::CancellationToken;

use super::LinkState;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long an ending the runner chose may take to send what is queued, its
/// close frame and to hear the backend's; a backend that takes longer sees
/// the connection end without them.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// The Host's log frames waiting to be written. The one queue on the link
/// with a length: its lines wait in the Host's log file, which holds them, and
/// a frame waits only for room here (`crates/runner/src/host_log.rs`).
const LOG_QUEUE: usize = 8;

/// What the reader hands the owner for one frame of the connection.
pub struct Arrival {
    /// The message, none for a numbered one that was not written again.
    pub message: Option<Inbound>,
    /// Whether the session numbered it: the owner counts it as received once
    /// it handled it ([`LinkState::handled`]), not before.
    pub numbered: bool,
}

/// What the writer may write: the hello alone, then nothing until the backend
/// answers it, then everything.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Greeting,
    Waiting,
    Open,
}

pub struct Transport {
    /// Job and process output, behind the control replies.
    pub output: mpsc::UnboundedSender<Frame>,
    /// Replies and requests, written first.
    pub control: mpsc::UnboundedSender<Frame>,
    /// The Host's log lines, written last: a command's exit or an answer
    /// leaves before a log frame (`crates/runner/src/host_log.rs`).
    pub log: mpsc::Sender<Frame>,
    pub input: mpsc::UnboundedReceiver<Arrival>,
    cancel: CancellationToken,
    owner: Option<JoinHandle<io::Result<()>>>,
}

/// The backend's runner WebSocket: its `ws`/`wss` form, at `/api/runner`
/// unless the URL names another path.
pub fn socket_url(backend: &Backend) -> io::Result<url::Url> {
    let mut url = backend.url().url().clone();
    let scheme = match backend.origin()?.scheme() {
        "http" => "ws",
        _ => "wss",
    };
    url.set_scheme(scheme)
        .map_err(|_| io::Error::other("invalid WebSocket scheme"))?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/api/runner");
    }
    Ok(url)
}

/// What asking for the socket gave: the socket, or the release the backend
/// requires first (`crates/runner/src/update.rs`).
pub enum Connected {
    Open(Transport),
    Update(RunnerUpdate),
}

impl Transport {
    /// Opens the backend's socket for a runner of `release`, which the
    /// backend checks before it opens it; a paired runner names its device
    /// with `token`.
    pub async fn connect(
        backend: &Backend,
        release: &str,
        token: Option<&DeviceToken>,
        link: LinkState,
        cancel: CancellationToken,
    ) -> io::Result<Connected> {
        let url = socket_url(backend)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(io::Error::other)?;
        let headers = request.headers_mut();
        headers.insert(
            RELEASE_HEADER,
            HeaderValue::from_str(release).map_err(io::Error::other)?,
        );
        headers.insert(TARGET_HEADER, HeaderValue::from_static(host_target()));
        if let Some(token) = token {
            let mut value = HeaderValue::from_str(token.expose()).map_err(io::Error::other)?;
            value.set_sensitive(true);
            headers.insert(TOKEN_HEADER, value);
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(wire::LINK_FRAME_BYTES))
            .max_frame_size(Some(wire::LINK_FRAME_BYTES));
        let connected = tokio::select! {
            _ = cancel.cancelled() => return Err(io::Error::new(io::ErrorKind::Interrupted, "connection cancelled")),
            result = tokio::time::timeout(HANDSHAKE_TIMEOUT, backend.watched_websocket(request, config)) => {
                result.map_err(io::Error::other)?
            }
        };
        match connected {
            Ok((socket, _, writes)) => Ok(Connected::Open(Self::from_socket(socket, writes, link, cancel))),
            Err(Error::Http(response)) if response.status() == StatusCode::CONFLICT => {
                let body = response.body().as_deref().unwrap_or_default();
                let update = RunnerUpdate::decode(body)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                Ok(Connected::Update(update))
            }
            Err(error) => Err(io::Error::other(error)),
        }
    }

    /// Serves `socket`, whose writes `writes` notes, as a connection of
    /// `link`'s session.
    pub fn from_socket<S>(
        socket: WebSocketStream<S>,
        writes: Arc<wire::WriteProgress>,
        link: LinkState,
        cancel: CancellationToken,
    ) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let cancel = cancel.child_token();
        let (output, mut outgoing) = mpsc::unbounded_channel::<Frame>();
        let (control, mut controls) = mpsc::unbounded_channel::<Frame>();
        let (log, mut logs) = mpsc::channel::<Frame>(LOG_QUEUE);
        let (incoming, input) = mpsc::unbounded_channel();
        let backlog = link.backlog().clone();
        let (opened, _) = watch::channel(false);
        let stopped = cancel.clone();
        let owner = tokio::spawn(async move {
            let (mut writer, mut reader) = socket.split();
            // Ends with the refusal of a message this side cannot decode,
            // which the connection then closes over.
            let receive = async {
                loop {
                    // The backend answers each ping, and pings itself.
                    let Ok(message) =
                        tokio::time::timeout(wire::LINK_SILENCE, reader.next()).await
                    else {
                        tracing::warn!(
                            event = "link.lost",
                            cause = "silence",
                            silent_ms = wire::LINK_SILENCE.as_millis() as u64,
                            "nothing arrived from the backend"
                        );
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "nothing arrived from the backend for {} seconds",
                                wire::LINK_SILENCE.as_secs()
                            ),
                        ));
                    };
                    let Some(message) = message else {
                        break;
                    };
                    match message.map_err(io::Error::other)? {
                        Message::Binary(bytes) => {
                            let read = link.session().read(&bytes);
                            let (payload, numbered) = match read.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))? {
                                Read::Own(payload) => (payload, false),
                                Read::Message(payload) => (payload, true),
                                // Nothing to hand on, but a skipped message
                                // counts in its turn, behind the ones before.
                                Read::Skipped => {
                                    if incoming.send(Arrival { message: None, numbered: true }).is_err() {
                                        return Ok(None);
                                    }
                                    continue;
                                }
                                Read::Duplicate | Read::Ack => {
                                    link.owe();
                                    continue;
                                }
                            };
                            let message = match wire::decode::<Inbound>(payload) {
                                Ok(message) => message,
                                Err(error) => {
                                    let refusal = wire::refusal(payload, &error);
                                    tracing::warn!(
                                        event = "message.refused",
                                        direction = "from_backend",
                                        kind = wire::refused_kind(&refusal),
                                        "a message of the backend could not be decoded"
                                    );
                                    return Ok(Some(refusal));
                                }
                            };
                            // The answer to the hello decides, before any
                            // message after it is read, whether the session
                            // goes on and what the writer writes again.
                            match &message {
                                Inbound::HelloOk { session, received, .. } => {
                                    link.hello_ok(*session, *received);
                                    opened.send_replace(true);
                                }
                                Inbound::Claimed { session, .. } => {
                                    link.claimed(*session);
                                    opened.send_replace(true);
                                }
                                _ => {}
                            }
                            if incoming.send(Arrival { message: Some(message), numbered }).is_err() {
                                return Ok(None);
                            }
                        }
                        // The backend closed it over a message of this
                        // runner's that it cannot decode, and says which.
                        Message::Close(Some(frame)) if frame.code == CloseCode::Invalid => {
                            tracing::warn!(
                                event = "message.refused",
                                direction = "to_backend",
                                kind = wire::refused_kind(&frame.reason),
                                "the backend could not decode a message of the runner"
                            );
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("the backend {}", frame.reason),
                            ));
                        }
                        Message::Close(_) => return Ok(None),
                        Message::Ping(_) | Message::Pong(_) => {}
                        _ => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "runner requires binary WebSocket messages",
                            ));
                        }
                    }
                }
                Ok::<_, io::Error>(None)
            };
            let send = async {
                let mut pings = tokio::time::interval_at(
                    tokio::time::Instant::now() + wire::RUNNER_PING_INTERVAL,
                    wire::RUNNER_PING_INTERVAL,
                );
                // Only the hello goes before the backend answers it, since
                // the answer says what the session carries on from.
                let mut phase = Phase::Greeting;
                let mut gate = opened.subscribe();
                loop {
                    let ack_due = link.session().ack_due();
                    let message = tokio::select! {
                        biased;
                        _ = pings.tick() => {
                            watched(&writes, writer.send(Message::Ping(Vec::new().into()))).await?;
                            continue;
                        }
                        changed = gate.changed(), if phase != Phase::Open => {
                            if changed.is_ok() && *gate.borrow() {
                                phase = Phase::Open;
                                // What the backend did not receive goes
                                // again, before anything new.
                                loop {
                                    let Some(frame) = link.session().next_resend() else { break };
                                    watched(&writes, writer.send(Message::Binary(frame.into()))).await?;
                                }
                            }
                            continue;
                        }
                        () = async {
                            match ack_due {
                                Some(due) => tokio::time::sleep_until(due).await,
                                None => std::future::pending().await,
                            }
                        } => {
                            let frame = link.session().ack_frame();
                            if let Some(frame) = frame {
                                watched(&writes, writer.send(Message::Binary(frame.into()))).await?;
                            }
                            continue;
                        }
                        () = link.owed(), if ack_due.is_none() => continue,
                        Some(message) = controls.recv(), if phase != Phase::Waiting => {
                            if phase == Phase::Greeting {
                                phase = Phase::Waiting;
                            }
                            message
                        }
                        message = backlog.next(), if phase == Phase::Open => message,
                        Some(message) = outgoing.recv(), if phase == Phase::Open => message,
                        Some(message) = logs.recv(), if phase == Phase::Open => message,
                        else => break,
                    };
                    write(&mut writer, &writes, &link, message).await?;
                }
                Ok::<_, io::Error>(())
            };
            // Both halves are owned by this task. Returning drops both futures
            // and the socket even when a write is stalled by a disconnected peer.
            let ended = tokio::select! {
                _ = stopped.cancelled() => None,
                result = receive => Some(result),
                result = send => Some(result.map(|()| None)),
            };
            match ended {
                // The connection closes over a message the runner cannot
                // decode, naming it, so the backend reports why it ended
                // (`crates/runner/src/connection/mod.rs`).
                Some(Ok(Some(refusal))) => {
                    let frame = CloseFrame {
                        code: CloseCode::Invalid,
                        reason: wire::close_reason(&refusal).into(),
                    };
                    // The connection ends with the refusal either way; a
                    // backend that is gone or slow only misses its reason.
                    let _sent =
                        tokio::time::timeout(CLOSE_TIMEOUT, writer.send(Message::Close(Some(frame))))
                            .await;
                    return Err(io::Error::new(io::ErrorKind::InvalidData, refusal));
                }
                Some(result) => return result.map(|_| ()),
                None => {}
            }
            // The runner ends this connection on purpose: what is queued
            // goes first, then a close frame, and the connection ends with
            // the backend's answer, so the backend sees the runner leave
            // rather than lose it (`crates/runner/src/connection/mod.rs`).
            let close = async {
                while let Ok(message) = controls.try_recv().or_else(|_| outgoing.try_recv()) {
                    write(&mut writer, &writes, &link, message).await?;
                }
                let frame = CloseFrame {
                    code: CloseCode::Away,
                    reason: "runner stopping".into(),
                };
                writer
                    .send(Message::Close(Some(frame)))
                    .await
                    .map_err(io::Error::other)?;
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
            log,
            input,
            cancel,
            owner: Some(owner),
        }
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

/// Writes one queued frame, numbered and kept by the session unless it belongs
/// to the connection. Replies over the limit already failed their requests;
/// anything else this large breaks the protocol.
async fn write<S>(
    writer: &mut S,
    writes: &wire::WriteProgress,
    link: &LinkState,
    message: Frame,
) -> io::Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let carriage = message.carriage();
    let bytes = message.into_bytes();
    if bytes.len() > wire::MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "runner outbound message exceeds {} bytes",
                wire::MAX_MESSAGE_BYTES
            ),
        ));
    }
    // Numbered before the write, so a write that fails halfway leaves the
    // message kept for the next connection.
    let frame = match carriage {
        wire::Carriage::Connection => wire::own_frame(bytes),
        wire::Carriage::Outlasting => link.session().stamp(bytes, true),
        wire::Carriage::Scoped => link.session().stamp(bytes, false),
    };
    watched(writes, writer.send(Message::Binary(frame.into()))).await
}

/// Runs a write, ending it once the socket's writes stall
/// ([`wire::WriteProgress::stalled`]).
async fn watched(
    writes: &wire::WriteProgress,
    sending: impl Future<Output = Result<(), tokio_tungstenite::tungstenite::Error>>,
) -> io::Result<()> {
    tokio::select! {
        sent = sending => sent.map_err(io::Error::other),
        () = writes.stalled() => Err(write_stalled()),
    }
}

/// The error of a write that made no progress for [`wire::LINK_WRITE_TIMEOUT`],
/// which ends the connection.
fn write_stalled() -> io::Error {
    tracing::warn!(
        event = "link.lost",
        cause = "write",
        stalled_ms = wire::LINK_WRITE_TIMEOUT.as_millis() as u64,
        "a write to the backend made no progress"
    );
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "a write made no progress for {} seconds",
            wire::LINK_WRITE_TIMEOUT.as_secs()
        ),
    )
}

impl Drop for Transport {
    fn drop(&mut self) {
        // Cancellation wakes the owner on every early return; explicit close joins it.
        self.cancel.cancel();
    }
}
