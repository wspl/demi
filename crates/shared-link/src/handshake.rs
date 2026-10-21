//! The socket before the session: the messages that come first, as ordinary
//! WebSocket messages, and the start of the yamux session once they are done.
//!
//! The runner's hello and the backend's answer to it are one message each,
//! as they were before the link, so a runner of an earlier version is
//! answered in the way it understands. The session starts on the same socket
//! after the answer that accepts the hello ([`Handshake::start`]). What is
//! sent on the link's [`Outgoing`] before then waits for the start.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use demi_runner_protocol::wire::MAX_MESSAGE_BYTES;
use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio_util::{compat::TokioAsyncReadCompatExt as _, sync::CancellationToken, task::TaskTracker};

use crate::{
    connection::{
        Connection, Delivery, Health, HealthKind, Incoming, Liveness, MESSAGE_BUDGET, MESSAGE_QUEUE,
        OUTBOUND_STREAMS, Owner, Shared, Side, WINDOW_BYTES, WINDOW_GROWTH, Command, drive, heartbeat,
        MAX_STREAMS,
    },
    outgoing::Outgoing,
    socket::{Heard, Received, Refusal, SocketEnd, SocketIo, Wire},
};

/// How long a close waits for the peer's close frame.
const PEER_CLOSE_WAIT: Duration = Duration::from_secs(5);

type Reader = Box<dyn Stream<Item = Result<Received, String>> + Unpin + Send>;
type Writer = Box<dyn Sink<Wire, Error = String> + Unpin + Send>;

/// A socket whose first messages are read and written by the application,
/// until it starts the session.
pub struct Handshake {
    owner: Arc<Owner>,
    parts: Option<Parts>,
}

struct Parts {
    reader: Reader,
    writer: Writer,
    refused: mpsc::UnboundedReceiver<String>,
    commanded: mpsc::UnboundedReceiver<Command>,
    delivered: mpsc::Receiver<Result<Delivery, SocketEnd>>,
    side: Side,
}

impl Handshake {
    /// Takes the socket's `incoming` messages and the `outgoing` sink it
    /// sends on.
    pub fn open<R, W, E>(incoming: R, outgoing: W, side: Side, liveness: Option<Liveness>) -> Self
    where
        R: Stream<Item = Result<Received, String>> + Unpin + Send + 'static,
        W: Sink<Wire, Error = E> + Unpin + Send + 'static,
        E: std::fmt::Display + 'static,
    {
        let (refusals, refused) = mpsc::unbounded_channel();
        let (messages, delivered) = mpsc::channel(MESSAGE_QUEUE);
        let (commands, commanded) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            liveness,
            commands,
            refusals,
            lanes: Mutex::default(),
            next_token: AtomicU64::new(0),
            budget: Arc::new(Semaphore::new(MESSAGE_BUDGET)),
            messages: Mutex::new(Some(messages)),
            closed: CancellationToken::new(),
            closing: CancellationToken::new(),
            abort: CancellationToken::new(),
            tasks: TaskTracker::new(),
            readers: TaskTracker::new(),
            outbound: Arc::new(Semaphore::new(OUTBOUND_STREAMS)),
            inbound: AtomicUsize::new(0),
            paused: AtomicBool::new(false),
            heard: Heard::new(),
            probe_now: Notify::new(),
            started: AtomicBool::new(false),
            end: Default::default(),
        });
        Self {
            owner: Arc::new(Owner(shared)),
            parts: Some(Parts {
                reader: Box::new(incoming),
                writer: Box::new(outgoing.sink_map_err(|error| error.to_string())),
                refused,
                commanded,
                delivered,
                side,
            }),
        }
    }

    /// Where the link sends once it runs: what is sent before waits.
    pub fn outgoing(&self) -> Outgoing {
        Outgoing::link(self.owner.clone())
    }

    /// The link's liveness, which answers every probe until the session runs.
    pub fn health(&self) -> Health {
        Health(HealthKind::Link(self.owner.clone()))
    }

    /// The next message of the socket, as its bytes; none when the peer
    /// closed it in order, and the reason when it did not.
    pub async fn read(&mut self) -> Option<Result<Vec<u8>, SocketEnd>> {
        let parts = self.parts.as_mut()?;
        loop {
            match parts.reader.next().await {
                Some(Ok(Received::Binary(bytes))) => {
                    if bytes.len() > MAX_MESSAGE_BYTES {
                        return Some(Err(SocketEnd::Broken(format!(
                            "a message of {} bytes, over the {MAX_MESSAGE_BYTES}-byte limit",
                            bytes.len()
                        ))));
                    }
                    return Some(Ok(bytes));
                }
                Some(Ok(Received::Nothing)) => {}
                Some(Ok(Received::Close(Some(Refusal(reason))))) => {
                    return Some(Err(SocketEnd::Refused(reason)));
                }
                Some(Ok(Received::Close(None))) | None => return None,
                Some(Err(error)) => return Some(Err(SocketEnd::Broken(error))),
            }
        }
    }

    /// Sends `message` as one message of the socket.
    pub async fn write(&mut self, message: Vec<u8>) -> Result<(), SocketEnd> {
        let Some(parts) = self.parts.as_mut() else {
            return Err(SocketEnd::Broken("the socket is closed".into()));
        };
        parts
            .writer
            .send(Wire::Binary(message))
            .await
            .map_err(SocketEnd::Broken)
    }

    /// Ends the socket in order: the close frame, with `refusal` when this
    /// end refuses a message, and a few seconds for the peer's.
    pub async fn close(mut self, refusal: Option<String>) {
        let Some(mut parts) = self.parts.take() else {
            return;
        };
        if parts.writer.send(Wire::Close(refusal)).await.is_ok() {
            let _ = tokio::time::timeout(PEER_CLOSE_WAIT, async {
                loop {
                    match parts.reader.next().await {
                        Some(Ok(Received::Close(_))) | Some(Err(_)) | None => return,
                        Some(Ok(_)) => {}
                    }
                }
            })
            .await;
        }
        self.owner.closed.cancel();
    }

    /// Starts the session on the socket, as yamux's server when this is the
    /// backend and its client when it is the runner. What was sent on
    /// [`Handshake::outgoing`] goes now.
    pub fn start(mut self) -> Connection {
        let Parts {
            reader,
            writer,
            refused,
            commanded,
            delivered,
            side,
        } = self.parts.take().expect("a handshake starts once");
        let shared = self.owner.0.clone();
        let io = SocketIo::new(reader, writer, refused, shared.heard.clone(), shared.end.clone());
        let mut config = yamux::Config::default();
        config
            .set_max_num_streams(MAX_STREAMS)
            .set_max_connection_receive_window(Some(MAX_STREAMS * WINDOW_BYTES + WINDOW_GROWTH));
        let connection = yamux::Connection::new(
            io.compat(),
            config,
            match side {
                Side::Runner => yamux::Mode::Client,
                Side::Backend => yamux::Mode::Server,
            },
        );
        shared.heard.restart();
        shared.started.store(true, Ordering::Release);
        tokio::spawn(drive(shared.clone(), connection, commanded));
        tokio::spawn(heartbeat(shared));
        Connection {
            outgoing: self.outgoing(),
            incoming: Incoming {
                messages: delivered,
                _owner: self.owner.clone(),
            },
            health: self.health(),
        }
    }
}

impl Drop for Handshake {
    fn drop(&mut self) {
        // A socket dropped before its session ran sends nothing more.
        if self.parts.is_some() {
            self.owner.closed.cancel();
        }
    }
}

impl Connection {
    /// Opens the link over a socket with no handshake: the session starts at
    /// once. It must run where tasks can be spawned.
    pub fn open<R, W, E>(incoming: R, outgoing: W, side: Side, liveness: Option<Liveness>) -> Self
    where
        R: Stream<Item = Result<Received, String>> + Unpin + Send + 'static,
        W: Sink<Wire, Error = E> + Unpin + Send + 'static,
        E: std::fmt::Display + 'static,
    {
        Handshake::open(incoming, outgoing, side, liveness).start()
    }
}
