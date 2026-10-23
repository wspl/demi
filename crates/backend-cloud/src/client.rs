//! The backend's end of the machine manager's socket
//! (`crates/machine-manager-protocol/src/lib.rs`).
//!
//! The client keeps one connection, opened by the first call and opened again
//! by the next call after it drops.
//!
//! - Requests carry ids this client counts, and replies are matched to them in
//!   whatever order they come.
//! - When the connection drops, every call in flight fails with a
//!   manager-unavailable error. Losing the socket does not establish that an
//!   operation failed, so the Cloud's lifecycle asks the manager before it
//!   retries.
//! - A death event goes to the one receiver the backend routes to the device's
//!   owner.
//! - A device's status the manager pushes (`ManagerEvent::CloudStatus`) is
//!   kept, the latest of each device, and its device goes to the receiver the
//!   backend routes to the device's owner. A page reads it from here and
//!   never calls the manager. The client connects without a call: it tries
//!   again every [`RECONNECT`] while it has no connection, so the manager's
//!   events arrive when it can.
//! - A line of the manager's log is stored in the backend's log as it arrives.
//!
//! A supervisor task owns the connection, the calls in flight and the id
//! counter. The client is its handle, and the task ends with the client.
//!
//! A connection serves calls once the manager's `hello` names this backend's
//! wire version. A manager of another release fails the call that connected.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use demi_machine_manager_protocol::{
    Capacity, DeviceId, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse,
    ManagerEvent, Operation, WIRE_VERSION, decode_response, encode_line,
};
use demi_backend_database::server_log::ServerLog;
use futures_util::StreamExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::task::AbortOnDropHandle;

/// Calls waiting for the supervisor before their callers wait to send more: a
/// slow manager slows its callers instead of growing memory.
const QUEUE: usize = 64;

/// Replies the reader has decoded ahead of the supervisor, bounded for the same
/// reason.
const INCOMING: usize = 64;

/// Death events waiting for the backend's router, bounded for the same reason.
pub(crate) const DEATHS: usize = 256;

/// Status changes waiting for the backend's router, bounded for the same
/// reason.
pub(crate) const STATUSES: usize = 256;

/// The pause before the client tries the manager again while it has no
/// connection: the manager may start after the backend.
const RECONNECT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a new connection waits for the manager's `hello`, which it
/// writes as it accepts.
const HELLO_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Why a call to the manager has no result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MachinesError {
    /// The manager could not be reached, or the connection dropped before it
    /// answered: whether the operation ran is not known.
    #[error("Machine manager unavailable during {operation}: {reason}")]
    Unavailable {
        operation: &'static str,
        reason: String,
    },
    /// The manager ran the operation and it failed.
    #[error("{0}")]
    Failed(String),
    /// The manager answered with a result the operation does not have.
    #[error("the machine manager answered {operation} with an unexpected result: {reason}")]
    Result {
        operation: &'static str,
        reason: String,
    },
}

/// A device's stored image as the manager last said it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageStatus {
    /// The capacities of its filesystems; none before its first generation.
    pub volumes: Option<Capacity>,
    /// Its system is on another base than the manager's configured one.
    pub newer_image: bool,
}

/// What the manager tells the backend unasked, for the backend's routers.
pub struct Notices {
    /// The device of each sandbox that exited without being asked to stop.
    pub deaths: mpsc::Receiver<DeviceId>,
    /// The device whose status changed.
    pub statuses: mpsc::Receiver<DeviceId>,
}

/// A handle on the machine manager's socket.
///
/// It is `Send + Sync`: a service every shard shares.
pub struct MachinesClient {
    commands: mpsc::Sender<Command>,
    statuses: Arc<Mutex<HashMap<DeviceId, ImageStatus>>>,
    /// Ends the supervisor with the client.
    _supervisor: AbortOnDropHandle<()>,
}

enum Command {
    Call {
        call: MachineCall,
        answer: oneshot::Sender<Result<serde_json::Value, MachinesError>>,
    },
    Disconnect {
        done: oneshot::Sender<()>,
    },
}

impl MachinesClient {
    /// A client of the socket at `socket`, on the runtime that calls this,
    /// and what the manager tells unasked ([`Notices`]). The lines of the
    /// manager's log that arrive on the connection are stored in `log`.
    pub fn new(socket: PathBuf, log: ServerLog) -> (Self, Notices) {
        let (commands, received) = mpsc::channel(QUEUE);
        let (deaths, died) = mpsc::channel(DEATHS);
        let (changed, statuses) = mpsc::channel(STATUSES);
        let kept = Arc::new(Mutex::new(HashMap::new()));
        let supervisor = Supervisor {
            socket,
            connection: None,
            pending: HashMap::new(),
            next_id: 1,
            deaths,
            changed,
            statuses: kept.clone(),
            log,
        };
        let task = tokio::spawn(supervisor.run(received));
        (
            Self {
                commands,
                statuses: kept,
                _supervisor: AbortOnDropHandle::new(task),
            },
            Notices { deaths: died, statuses },
        )
    }

    /// What the manager last said of `device`'s stored image; none before it
    /// said anything of it. It calls no one.
    pub fn status(&self, device: &DeviceId) -> Option<ImageStatus> {
        self.statuses
            .lock()
            .expect("the statuses are never held across a panic")
            .get(device)
            .copied()
    }

    /// Runs `params` on the manager and answers its result.
    pub async fn call<O: Operation>(&self, params: O) -> Result<O::Output, MachinesError> {
        let call: MachineCall = params.into();
        let operation = call.name();
        let (answer, answered) = oneshot::channel();
        let unavailable = || MachinesError::Unavailable {
            operation,
            reason: "the machine manager's client is closed".into(),
        };
        self.commands
            .send(Command::Call { call, answer })
            .await
            .map_err(|_| unavailable())?;
        let result = answered.await.map_err(|_| unavailable())??;
        serde_json::from_value(result).map_err(|error| MachinesError::Result {
            operation,
            reason: error.to_string(),
        })
    }

    /// Disconnects, leaving the manager and its sandboxes running
    /// (`crates/machine-manager-protocol/src/lib.rs`); a later call
    /// connects again.
    pub async fn close(&self) {
        let (done, disconnected) = oneshot::channel();
        if self
            .commands
            .send(Command::Disconnect { done })
            .await
            .is_ok()
        {
            // The supervisor answers once the connection is gone; if it
            // ended, so did the connection.
            let _ = disconnected.await;
        }
    }
}

/// The connection and the calls in flight on it.
struct Supervisor {
    socket: PathBuf,
    connection: Option<Connection>,
    pending: HashMap<String, Pending>,
    next_id: u64,
    deaths: mpsc::Sender<DeviceId>,
    changed: mpsc::Sender<DeviceId>,
    statuses: Arc<Mutex<HashMap<DeviceId, ImageStatus>>>,
    log: ServerLog,
}

struct Connection {
    writer: OwnedWriteHalf,
    incoming: mpsc::Receiver<Incoming>,
    /// Reads the socket; it ends with the connection.
    _reader: AbortOnDropHandle<()>,
}

/// What the reader saw on the socket.
enum Incoming {
    Line(String),
    /// The socket closed or broke, for this reason.
    Closed(String),
}

struct Pending {
    operation: &'static str,
    started: std::time::Instant,
    answer: oneshot::Sender<Result<serde_json::Value, MachinesError>>,
}

impl Pending {
    /// The line that ends a call to the machine manager.
    fn log_ended(&self, outcome: &'static str) {
        tracing::info!(
            event = "machine_manager.call.ended",
            operation = self.operation,
            outcome,
            ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "a call to the machine manager ended"
        );
    }
}

impl Supervisor {
    async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        let mut reconnect = tokio::time::interval(RECONNECT);
        reconnect.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = reconnect.tick(), if self.connection.is_none() => {
                    match connect(&self.socket).await {
                        Ok(connection) => self.connection = Some(connection),
                        // The manager may not run yet; a call that needs it
                        // fails on its own.
                        Err(reason) => tracing::debug!(event = "machine_manager.connect_failed", reason, "the machine manager could not be reached"),
                    }
                }
                command = commands.recv() => match command {
                    // The client is gone, and the connection goes with it.
                    None => return,
                    Some(Command::Call { call, answer }) => self.call(call, answer).await,
                    Some(Command::Disconnect { done }) => {
                        self.dropped("the backend disconnected");
                        // A closer that went away needs no answer.
                        let _ = done.send(());
                    }
                },
                incoming = next_incoming(&mut self.connection) => match incoming {
                    Incoming::Line(line) => self.receive(&line).await,
                    Incoming::Closed(reason) => self.dropped(&reason),
                },
            }
        }
    }

    /// Sends `call`, connecting first.
    async fn call(
        &mut self,
        call: MachineCall,
        answer: oneshot::Sender<Result<serde_json::Value, MachinesError>>,
    ) {
        let operation = call.name();
        if self.connection.is_none() {
            match connect(&self.socket).await {
                Ok(connection) => self.connection = Some(connection),
                Err(reason) => {
                    // A caller that went away needs no answer.
                    let _ = answer.send(Err(MachinesError::Unavailable { operation, reason }));
                    tracing::info!(event = "machine_manager.call.ended", operation, outcome = "unavailable", ms = 0u64, "a call to the machine manager ended");
                    return;
                }
            }
        }
        let id = self.next_id.to_string();
        self.next_id += 1;
        let line = encode_line(&MachineRequest {
            id: id.clone(),
            call,
        });
        self.pending.insert(id, Pending { operation, started: std::time::Instant::now(), answer });
        let connection = self
            .connection
            .as_mut()
            .expect("the connection was just opened");
        if let Err(error) = connection.writer.write_all(&line).await {
            self.dropped(&error.to_string());
        }
    }

    /// Answers the call a reply names, or routes a death.
    async fn receive(&mut self, line: &str) {
        let response = match decode_response(line) {
            Ok(response) => response,
            Err(error) => {
                // The manager's other replies are still readable.
                tracing::warn!(event = "machine_manager.line_dropped", %error, "an unreadable line from the machine manager was dropped");
                return;
            }
        };
        let (id, result) = match response {
            MachineResponse::Hello { .. } => {
                tracing::warn!(event = "machine_manager.hello_repeated", "the machine manager sent a second hello, which was dropped");
                return;
            }
            MachineResponse::Death { device_id } => {
                match DeviceId::parse(device_id) {
                    // The router only ends when the backend does, which ends
                    // this client first; a death it cannot take has no one
                    // left to tell.
                    Ok(device) => {
                        let _ = self.deaths.send(device).await;
                    }
                    Err(error) => {
                        tracing::warn!(event = "machine_manager.sandbox_died", reason = "device_id_unreadable", %error, "the machine manager reported the death of a sandbox that is no device")
                    }
                }
                return;
            }
            MachineResponse::Log { line } => {
                self.log.store_manager(line);
                return;
            }
            MachineResponse::Event { event } => {
                self.event(event).await;
                return;
            }
            MachineResponse::Ok { id, result } => (id, Ok(result)),
            MachineResponse::Error { id, message } => (id, Err(MachinesError::Failed(message))),
        };
        match self.pending.remove(&id) {
            Some(pending) => {
                pending.log_ended(if result.is_ok() { "ok" } else { "failed" });
                // A caller that went away reads no answer; the operation
                // ran all the same.
                let _ = pending.answer.send(result);
            }
            None => tracing::warn!(
                event = "machine_manager.answer_unmatched",
                call_id = id,
                "the machine manager answered a request this backend did not send"
            ),
        }
    }

    /// Keeps what the manager said of a device, and tells the router.
    async fn event(&mut self, event: ManagerEvent) {
        let ManagerEvent::CloudStatus { device_id, volumes, newer_image } = event;
        let device = match DeviceId::parse(device_id) {
            Ok(device) => device,
            Err(error) => {
                tracing::warn!(event = "machine_manager.status_dropped", %error, "the machine manager reported the status of a device no id names");
                return;
            }
        };
        let status = ImageStatus { volumes, newer_image };
        let changed = self
            .statuses
            .lock()
            .expect("the statuses are never held across a panic")
            .insert(device.clone(), status)
            != Some(status);
        if changed {
            // The router only ends when the backend does, which ends this
            // client first.
            let _ = self.changed.send(device).await;
        }
    }

    /// The connection is gone: every call in flight fails, and the next
    /// call connects again.
    fn dropped(&mut self, reason: &str) {
        self.connection = None;
        if !self.pending.is_empty() {
            tracing::warn!(
                event = "machine_manager.connection_dropped",
                calls = self.pending.len(),
                reason,
                "the machine manager's connection dropped"
            );
        }
        for (_, pending) in self.pending.drain() {
            pending.log_ended("unavailable");
            let error = MachinesError::Unavailable {
                operation: pending.operation,
                reason: reason.to_owned(),
            };
            let _ = pending.answer.send(Err(error));
        }
    }
}

/// The next thing the reader saw, or never while no connection is open.
async fn next_incoming(connection: &mut Option<Connection>) -> Incoming {
    match connection {
        Some(connection) => connection
            .incoming
            .recv()
            .await
            .unwrap_or_else(|| Incoming::Closed("the connection's reader ended".into())),
        None => std::future::pending().await,
    }
}

/// Opens the socket and starts reading its lines.
async fn connect(socket: &Path) -> Result<Connection, String> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|error| format!("{}: {error}", socket.display()))?;
    let (read, writer) = stream.into_split();
    let mut framed = FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE_BYTES));
    let hello = tokio::time::timeout(HELLO_WAIT, framed.next())
        .await
        .map_err(|_| "the machine manager sent no hello".to_owned())?;
    let version = match hello.map(|line| line.map(|line| decode_response(&line))) {
        Some(Ok(Ok(MachineResponse::Hello { version }))) => version,
        Some(Ok(Ok(_))) => return Err("the machine manager's first message was no hello".into()),
        Some(Ok(Err(error))) => return Err(format!("the machine manager's hello did not read: {error}")),
        Some(Err(error)) => return Err(error.to_string()),
        None => return Err("the machine manager closed the connection".into()),
    };
    if version != WIRE_VERSION {
        return Err(format!(
            "the machine manager speaks wire version {version} and this backend {WIRE_VERSION}: start both from one release"
        ));
    }
    let (lines, incoming) = mpsc::channel(INCOMING);
    let reader = tokio::spawn(async move {
        let end = loop {
            match framed.next().await {
                Some(Ok(line)) if line.is_empty() => {}
                Some(Ok(line)) => {
                    if lines.send(Incoming::Line(line)).await.is_err() {
                        return;
                    }
                }
                Some(Err(error)) => break error.to_string(),
                None => break "the machine manager closed the connection".to_owned(),
            }
        };
        // The supervisor may have let go of this connection already.
        let _ = lines.send(Incoming::Closed(end)).await;
    });
    Ok(Connection {
        writer,
        incoming,
        _reader: AbortOnDropHandle::new(reader),
    })
}
