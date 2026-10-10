//! The backend's end of the machine manager's socket (`managed-hosts.md`
//! § Control and ownership): one connection, opened by the first call and
//! opened again by the next call after it drops. Requests carry ids this
//! client counts, and replies are matched to them in whatever order they
//! come. When the connection drops, every call in flight fails with a
//! manager-unavailable error: losing the socket does not establish that an
//! operation failed, so the Cloud's lifecycle asks the manager before it
//! retries. A death event goes to the one receiver the backend routes to
//! the device's owner.
//!
//! A supervisor task owns the connection, the calls in flight and the id
//! counter; the client is its handle, and the task ends with the client.
//! A connection serves calls once the manager's `hello` names this
//! backend's wire version; a manager of another release fails the call that
//! connected.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use demi_machine_manager_protocol::{
    DeviceId, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, Operation,
    WIRE_VERSION, decode_response, encode_line,
};
use futures_util::StreamExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::task::AbortOnDropHandle;

/// Calls waiting for the supervisor before their callers wait to send more.
const QUEUE: usize = 64;

/// Replies the reader has decoded ahead of the supervisor.
const INCOMING: usize = 64;

/// Death events waiting for the backend's router.
pub(crate) const DEATHS: usize = 256;

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

/// A handle on the machine manager's socket. `Send + Sync`: it is a service
/// every shard shares. The `testing` feature exports it for the Cloud suite,
/// which asks a real manager for a checkpoint and a device's generation.
pub struct MachinesClient {
    commands: mpsc::Sender<Command>,
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
    /// and the receiver of the manager's death events: the device of each
    /// sandbox that exited without being asked to stop.
    pub fn new(socket: PathBuf) -> (Self, mpsc::Receiver<DeviceId>) {
        let (commands, received) = mpsc::channel(QUEUE);
        let (deaths, died) = mpsc::channel(DEATHS);
        let supervisor = Supervisor {
            socket,
            connection: None,
            pending: HashMap::new(),
            next_id: 1,
            deaths,
        };
        let task = tokio::spawn(supervisor.run(received));
        (
            Self {
                commands,
                _supervisor: AbortOnDropHandle::new(task),
            },
            died,
        )
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
    /// (`managed-hosts.md` § Control and ownership); a later call connects
    /// again.
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
    answer: oneshot::Sender<Result<serde_json::Value, MachinesError>>,
}

impl Supervisor {
    async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        loop {
            tokio::select! {
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
        self.pending.insert(id, Pending { operation, answer });
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
                tracing::warn!("an unreadable line from the machine manager was dropped: {error}");
                return;
            }
        };
        let (id, result) = match response {
            MachineResponse::Hello { .. } => {
                tracing::warn!("the machine manager sent a second hello, which was dropped");
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
                        tracing::warn!("the machine manager reported the death of {error}")
                    }
                }
                return;
            }
            MachineResponse::Ok { id, result } => (id, Ok(result)),
            MachineResponse::Error { id, message } => (id, Err(MachinesError::Failed(message))),
        };
        match self.pending.remove(&id) {
            Some(pending) => {
                // A caller that went away reads no answer; the operation
                // ran all the same.
                let _ = pending.answer.send(result);
            }
            None => tracing::warn!(
                id,
                "the machine manager answered a request this backend did not send"
            ),
        }
    }

    /// The connection is gone: every call in flight fails, and the next
    /// call connects again.
    fn dropped(&mut self, reason: &str) {
        self.connection = None;
        if !self.pending.is_empty() {
            tracing::warn!(
                calls = self.pending.len(),
                "the machine manager's connection dropped: {reason}"
            );
        }
        for (_, pending) in self.pending.drain() {
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
