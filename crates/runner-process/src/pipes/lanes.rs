//! The runner's end of its pipe connection
//! (`crates/runner-protocol/src/wire/pipe_connection.rs`): one WebSocket to
//! the backend whose lanes carry the stream pipes this runner writes, so a
//! pipe opens with one frame instead of a connection of its own, which
//! through a proxy and a TLS edge costs two or three round trips.
//!
//! The connection opens as the runner's link comes online, or when a pipe
//! needs it first, and again after it ends. A pipe's first messages follow
//! its open at once, within the lane's credit; the backend answers a refused
//! open with the words a refused pipe request has. The connection's end
//! fails every pipe it carries, as a pipe's own connection lost would.
//!
//! ```text
//!  pipe --> LaneSender --- open, data, close ---> writer --> backend
//!  pipe <-- LaneReceiver <-- sink, close, refusal --- reader <-- backend
//!                     credit, from the reader, paces each LaneSender
//! ```

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use demi_runner_protocol::{
    values::DeviceToken,
    wire::{self, PIPE_CONNECTION_ROUTE, PIPE_LANE_CREDIT, PipeLaneRefusal, PipeLaneTarget, STREAM_PIPE_MESSAGE_BYTES},
};
use demi_shared_lanes::{Assembly, Frame, LANE_PIECE_BYTES, Message as LaneMessage, pieces};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::{Semaphore, mpsc, watch};
use tokio_tungstenite::tungstenite::{
    self, Message,
    client::IntoClientRequest,
    http::header::AUTHORIZATION,
    protocol::WebSocketConfig,
};
use tokio_util::sync::CancellationToken;

use super::{Attempt, reconnecting, refused, socket_error};
use crate::backend::Backend;

/// The most bytes of one frame the backend sends: a piece and its header,
/// or a close and its reason, with room to spare. Bounded because a message
/// is read whole into memory.
const MAX_FRAME_BYTES: usize = LANE_PIECE_BYTES + 1024;

/// The runner's pipe connection, opened again whenever it ended.
pub(crate) struct Lanes {
    backend: Backend,
    url: String,
    token: watch::Receiver<Option<DeviceToken>>,
    connect_timeout: Duration,
    current: tokio::sync::Mutex<Option<Arc<Connection>>>,
}

struct Connection {
    writes: mpsc::UnboundedSender<Bytes>,
    lanes: Mutex<HashMap<u32, LaneEnd>>,
    /// The number of the next lane, from 1.
    next: AtomicU32,
    ended: CancellationToken,
}

/// What the reader keeps of an open lane.
struct LaneEnd {
    heard: mpsc::UnboundedSender<io::Result<Heard>>,
    credit: Arc<Semaphore>,
}

/// What a pipe hears from the backend on its lane.
pub(crate) enum Heard {
    Text(String),
    Closed { code: u16, reason: String },
}

impl Lanes {
    /// The pipe connection to `backend`, whose `origin` it is, with the
    /// device token `token` holds, each connection taking at most
    /// `connect_timeout` to open.
    pub(crate) fn new(
        backend: &Backend,
        origin: &reqwest::Url,
        token: watch::Receiver<Option<DeviceToken>>,
        connect_timeout: Duration,
    ) -> io::Result<Self> {
        let mut url = origin.join(PIPE_CONNECTION_ROUTE).map_err(io::Error::other)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| io::Error::other("invalid WebSocket scheme"))?;
        Ok(Self {
            backend: backend.clone(),
            url: url.to_string(),
            token,
            connect_timeout,
            current: tokio::sync::Mutex::new(None),
        })
    }

    /// Opens a lane that carries what `target` names: its first messages may
    /// follow at once.
    pub(crate) async fn open(&self, target: &PipeLaneTarget) -> io::Result<(LaneSender, LaneReceiver)> {
        let connection = self.connection().await?;
        let lane = connection.next.fetch_add(1, Ordering::Relaxed);
        let (heard, hearing) = mpsc::unbounded_channel();
        let credit = Arc::new(Semaphore::new(usize::from(PIPE_LANE_CREDIT)));
        connection.lanes.lock().expect("the lanes are whole").insert(
            lane,
            LaneEnd {
                heard,
                credit: credit.clone(),
            },
        );
        // The reader cancels `ended` before it lets go of the lanes, so a lane
        // that came after hears it here.
        if connection.ended.is_cancelled() {
            connection.lanes.lock().expect("the lanes are whole").remove(&lane);
            return Err(lost());
        }
        let target = Bytes::from(serde_json::to_vec(target).expect("a lane's target serializes"));
        connection.write(Frame::Open { lane, target })?;
        let sender = LaneSender {
            lane,
            connection,
            credit,
            closed: false,
        };
        Ok((sender, LaneReceiver(hearing)))
    }

    /// Opens the connection in the background unless it is open, so the
    /// first pipe waits for no handshake.
    pub(crate) fn warm(self: &Arc<Self>) {
        let lanes = self.clone();
        tokio::spawn(async move {
            if let Err(error) = lanes.connection().await {
                tracing::debug!(event = "pipe_connection.failed", %error, "the pipe connection did not open");
            }
        });
    }

    /// The open connection, opened now unless one is.
    async fn connection(&self) -> io::Result<Arc<Connection>> {
        let mut current = self.current.lock().await;
        if let Some(connection) = current.as_ref()
            && !connection.ended.is_cancelled()
        {
            return Ok(connection.clone());
        }
        let mut request = self.url.as_str().into_client_request().map_err(io::Error::other)?;
        if let Some(authorization) = super::authorization(&self.token)? {
            request.headers_mut().insert(AUTHORIZATION, authorization);
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME_BYTES))
            .max_frame_size(Some(MAX_FRAME_BYTES));
        // Out of open files, the connection waits for one (`crates/command-sdk/src/descriptors.rs`).
        let never = CancellationToken::new();
        let (socket, _) = reconnecting(|| async {
            let connecting = demi_command_sdk::descriptors::retry(&never, || async {
                self.backend
                    .websocket(request.clone(), config)
                    .await
                    .map_err(|error| match error {
                        // A refused handshake reached the backend.
                        tungstenite::Error::Http(_) => Attempt::Refused(socket_error(error)),
                        error => Attempt::Lost(socket_error(error)),
                    })
            });
            tokio::time::timeout(self.connect_timeout, connecting)
                .await
                .unwrap_or_else(|_| Err(Attempt::Lost(io::Error::new(io::ErrorKind::TimedOut, "pipe connection did not open"))))
        })
        .await?;
        let (sink, stream) = socket.split();
        let (writes, writing) = mpsc::unbounded_channel();
        let connection = Arc::new(Connection {
            writes,
            lanes: Mutex::default(),
            next: AtomicU32::new(1),
            ended: CancellationToken::new(),
        });
        tokio::spawn(write(sink, writing, connection.ended.clone()));
        tokio::spawn(read(stream, connection.clone()));
        *current = Some(connection.clone());
        Ok(connection)
    }
}

impl Connection {
    fn write(&self, frame: Frame) -> io::Result<()> {
        self.writes.send(frame.encode()).map_err(|_| lost())
    }

    /// The lane `lane`, which the backend ends.
    fn take(&self, lane: u32) -> Option<LaneEnd> {
        self.lanes.lock().expect("the lanes are whole").remove(&lane)
    }
}

/// Writes the lanes' frames, and a ping after [`wire::PIPE_IDLE`] without
/// one, which keeps the connection through an edge that ends a silent one.
async fn write<S>(mut sink: S, mut writing: mpsc::UnboundedReceiver<Bytes>, ended: CancellationToken)
where
    S: futures_util::Sink<Message, Error = tungstenite::Error> + Unpin,
{
    let mut wrote_at = tokio::time::Instant::now();
    loop {
        let message = tokio::select! {
            () = ended.cancelled() => break,
            frame = writing.recv() => match frame {
                Some(frame) => Message::Binary(frame),
                None => break,
            },
            () = tokio::time::sleep_until(wrote_at + wire::PIPE_IDLE) => Message::Ping(Bytes::new()),
        };
        if sink.send(message).await.is_err() {
            break;
        }
        wrote_at = tokio::time::Instant::now();
    }
    ended.cancel();
}

/// Reads the backend's frames until the connection ends, then fails every
/// lane still open.
async fn read<S>(mut stream: S, connection: Arc<Connection>)
where
    S: futures_util::Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
{
    let mut assembling: HashMap<u32, Assembly> = HashMap::new();
    loop {
        let message = tokio::select! {
            () = connection.ended.cancelled() => break,
            message = stream.next() => message,
        };
        let frame = match message {
            Some(Ok(Message::Binary(bytes))) => Frame::decode(bytes),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => break,
        };
        match frame {
            Ok(Frame::Opened { .. } | Frame::Connection(_)) => {}
            Ok(Frame::Refused { lane, refusal }) => {
                let refusal = serde_json::from_slice::<PipeLaneRefusal>(&refusal).unwrap_or_else(|error| PipeLaneRefusal {
                    status: 502,
                    message: format!("the backend's refusal was not understood: {error}"),
                });
                if let Some(end) = connection.take(lane) {
                    let status = reqwest::StatusCode::from_u16(refusal.status).unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
                    let _ = end.heard.send(Err(refused(status, refusal.message.as_bytes())));
                }
            }
            Ok(Frame::Data { lane, flags, piece }) => {
                let message = assembling
                    .entry(lane)
                    .or_default()
                    .take(flags, piece, STREAM_PIPE_MESSAGE_BYTES);
                match message {
                    Ok(Some(LaneMessage::Text(text))) => {
                        let lanes = connection.lanes.lock().expect("the lanes are whole");
                        if let Some(end) = lanes.get(&lane) {
                            // A pipe that ended hears nothing more.
                            let _ = end.heard.send(Ok(Heard::Text(text)));
                        }
                    }
                    // The backend sends a source no bytes.
                    Ok(Some(LaneMessage::Binary(_)) | None) => {}
                    Err(_) => break,
                }
            }
            Ok(Frame::Close { lane, code, reason }) => {
                assembling.remove(&lane);
                if let Some(end) = connection.take(lane) {
                    let _ = end.heard.send(Ok(Heard::Closed { code, reason }));
                }
            }
            Ok(Frame::Credit { lane, messages }) => {
                let lanes = connection.lanes.lock().expect("the lanes are whole");
                if let Some(end) = lanes.get(&lane) {
                    end.credit.add_permits(usize::from(messages));
                }
            }
            // The backend broke the format: the connection is of no use.
            Ok(Frame::Open { .. }) | Err(_) => break,
        }
    }
    connection.ended.cancel();
    let lanes = std::mem::take(&mut *connection.lanes.lock().expect("the lanes are whole"));
    for end in lanes.into_values() {
        let _ = end.heard.send(Err(lost()));
    }
}

/// What a pipe sends on its lane, as the lane's credit lets it.
pub(crate) struct LaneSender {
    lane: u32,
    connection: Arc<Connection>,
    credit: Arc<Semaphore>,
    /// Whether the pipe said its end.
    closed: bool,
}

impl LaneSender {
    pub(crate) async fn binary(&mut self, bytes: Bytes) -> io::Result<()> {
        self.message(true, bytes).await
    }

    pub(crate) async fn text(&mut self, text: String) -> io::Result<()> {
        self.message(false, Bytes::from(text)).await
    }

    /// Ends the pipe, with `code` and `reason` as a WebSocket's close frame
    /// would say it, its reason cut to the frame's room, so a close always
    /// fits a frame the backend takes.
    pub(crate) fn close(&mut self, code: u16, reason: &str) -> io::Result<()> {
        self.closed = true;
        self.connection.write(Frame::Close {
            lane: self.lane,
            code,
            reason: wire::close_reason(reason).to_owned(),
        })
    }

    async fn message(&mut self, binary: bool, bytes: Bytes) -> io::Result<()> {
        let permit = tokio::select! {
            permit = self.credit.acquire() => permit.map_err(|_| lost())?,
            () = self.connection.ended.cancelled() => return Err(lost()),
        };
        permit.forget();
        for piece in pieces(self.lane, binary, bytes) {
            self.connection.write(piece)?;
        }
        Ok(())
    }
}

impl Drop for LaneSender {
    /// A pipe that let go without its end fails, as a WebSocket dropped
    /// without a close frame fails it.
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.close(u16::from(tungstenite::protocol::frame::coding::CloseCode::Error), "the pipe's source let go");
        }
    }
}

/// What a pipe hears on its lane.
pub(crate) struct LaneReceiver(mpsc::UnboundedReceiver<io::Result<Heard>>);

impl LaneReceiver {
    /// The next thing the backend says; an error once the lane was refused or
    /// its connection ended.
    pub(crate) async fn recv(&mut self) -> io::Result<Heard> {
        self.0.recv().await.unwrap_or_else(|| Err(lost()))
    }
}

fn lost() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "the pipe connection ended")
}
