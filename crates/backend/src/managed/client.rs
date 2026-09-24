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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use demi_machines_protocol::{
    DeviceId, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, Operation, ReconcileParams,
    decode_response, encode_line,
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

/// Why a call to the manager has no result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum MachinesError {
    /// The manager could not be reached, or the connection dropped before it
    /// answered: whether the operation ran is not known.
    #[error("Machine manager unavailable during {operation}: {reason}")]
    Unavailable { operation: &'static str, reason: String },
    /// The manager ran the operation and it failed.
    #[error("{0}")]
    Failed(String),
    /// The manager answered with a result the operation does not have.
    #[error("the machine manager answered {operation} with an unexpected result: {reason}")]
    Result { operation: &'static str, reason: String },
}

/// A handle on the machine manager's socket. `Send + Sync`: it is a service
/// every shard shares.
pub(crate) struct MachinesClient {
    commands: mpsc::Sender<Command>,
    /// Ends the supervisor with the client.
    _supervisor: AbortOnDropHandle<()>,
}

enum Command {
    Call {
        call: MachineCall,
        /// Only over the connection that is open now; none is opened for it.
        live_only: bool,
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
    pub(crate) fn new(socket: PathBuf) -> (Self, mpsc::Receiver<DeviceId>) {
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
    pub(crate) async fn call<O: Operation>(&self, params: O) -> Result<O::Output, MachinesError> {
        self.send(params, false).await
    }

    /// Reconciles the manager over the connection that is open, logging a
    /// failure, and disconnects; a client that never connected has nothing
    /// to do. The manager keeps running, and a later call connects again.
    pub(crate) async fn close(&self) -> Result<(), MachinesError> {
        let reconciled = match self.send(ReconcileParams {}, true).await {
            Err(MachinesError::Unavailable { reason, .. }) if reason == NOT_CONNECTED => Ok(()),
            reconciled => reconciled,
        };
        let (done, disconnected) = oneshot::channel();
        if self.commands.send(Command::Disconnect { done }).await.is_ok() {
            // The supervisor answers once the connection is gone; if it
            // ended, so did the connection.
            let _ = disconnected.await;
        }
        reconciled
    }

    async fn send<O: Operation>(&self, params: O, live_only: bool) -> Result<O::Output, MachinesError> {
        let call: MachineCall = params.into();
        let operation = call.name();
        let (answer, answered) = oneshot::channel();
        let unavailable = || MachinesError::Unavailable {
            operation,
            reason: "the machine manager's client is closed".into(),
        };
        self.commands
            .send(Command::Call { call, live_only, answer })
            .await
            .map_err(|_| unavailable())?;
        let result = answered.await.map_err(|_| unavailable())??;
        serde_json::from_value(result).map_err(|error| MachinesError::Result {
            operation,
            reason: error.to_string(),
        })
    }
}

/// Why a call over the live connection found none.
const NOT_CONNECTED: &str = "no connection is open";

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
                    Some(Command::Call { call, live_only, answer }) => self.call(call, live_only, answer).await,
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

    /// Sends `call`, connecting first unless `live_only`.
    async fn call(
        &mut self,
        call: MachineCall,
        live_only: bool,
        answer: oneshot::Sender<Result<serde_json::Value, MachinesError>>,
    ) {
        let operation = call.name();
        if self.connection.is_none() {
            if live_only {
                let reason = NOT_CONNECTED.to_owned();
                // A caller that went away needs no answer.
                let _ = answer.send(Err(MachinesError::Unavailable { operation, reason }));
                return;
            }
            match connect(&self.socket).await {
                Ok(connection) => self.connection = Some(connection),
                Err(reason) => {
                    let _ = answer.send(Err(MachinesError::Unavailable { operation, reason }));
                    return;
                }
            }
        }
        let id = self.next_id.to_string();
        self.next_id += 1;
        let line = encode_line(&MachineRequest { id: id.clone(), call });
        self.pending.insert(id, Pending { operation, answer });
        let connection = self.connection.as_mut().expect("the connection was just opened");
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
            MachineResponse::Death { device_id } => {
                match DeviceId::parse(device_id) {
                    // The router only ends when the backend does, which ends
                    // this client first; a death it cannot take has no one
                    // left to tell.
                    Ok(device) => {
                        let _ = self.deaths.send(device).await;
                    }
                    Err(error) => tracing::warn!("the machine manager reported the death of {error}"),
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
            None => tracing::warn!(id, "the machine manager answered a request this backend did not send"),
        }
    }

    /// The connection is gone: every call in flight fails, and the next
    /// call connects again.
    fn dropped(&mut self, reason: &str) {
        self.connection = None;
        if !self.pending.is_empty() {
            tracing::warn!(calls = self.pending.len(), "the machine manager's connection dropped: {reason}");
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
    let (lines, incoming) = mpsc::channel(INCOMING);
    let reader = tokio::spawn(async move {
        let mut framed = FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE_BYTES));
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use demi_machines_protocol::{
        BaseVersion, CurrentBaseVersionParams, HibernateParams, ImageStateParams, MachineImageState, decode_request,
    };
    use tokio::io::{AsyncBufReadExt as _, BufReader};
    use tokio::net::UnixListener;

    use super::*;

    /// One connection of a peer the test scripts: the requests it read, and
    /// the lines it writes back.
    struct Peer {
        lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
        writer: OwnedWriteHalf,
    }

    impl Peer {
        async fn accept(listener: &UnixListener) -> Self {
            let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .expect("the client connects")
                .unwrap();
            let (read, writer) = stream.into_split();
            Self {
                lines: BufReader::new(read).lines(),
                writer,
            }
        }

        async fn request(&mut self) -> MachineRequest {
            let line = self.lines.next_line().await.unwrap().expect("a request");
            decode_request(&line).unwrap()
        }

        async fn write(&mut self, response: &MachineResponse) {
            self.writer.write_all(&encode_line(response)).await.unwrap();
        }

        async fn write_raw(&mut self, line: &str) {
            self.writer.write_all(line.as_bytes()).await.unwrap();
        }

        async fn ok(&mut self, id: &str, result: serde_json::Value) {
            let response = MachineResponse::Ok { id: id.into(), result };
            self.write(&response).await;
        }

        /// Whether the client closed the connection.
        async fn closed(&mut self) -> bool {
            tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
                .await
                .is_ok_and(|line| matches!(line, Ok(None)))
        }
    }

    fn socket() -> (tempfile::TempDir, PathBuf, UnixListener) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("machines.sock");
        let listener = UnixListener::bind(&path).unwrap();
        (directory, path, listener)
    }

    fn device(name: &str) -> String {
        name.to_owned()
    }

    #[tokio::test]
    async fn calls_cross_the_socket_and_their_replies_are_matched_by_id_whatever_their_order() {
        let (_directory, path, listener) = socket();
        let (client, _deaths) = MachinesClient::new(path);
        let client = std::sync::Arc::new(client);
        // Nothing connects before the first call.
        assert!(tokio::time::timeout(Duration::from_millis(50), listener.accept()).await.is_err());
        let calls = {
            let client = client.clone();
            tokio::spawn(async move {
                tokio::join!(
                    client.call(CurrentBaseVersionParams {}),
                    client.call(ImageStateParams { device_id: device("dev-1") }),
                )
            })
        };
        let mut peer = Peer::accept(&listener).await;
        let first = peer.request().await;
        let second = peer.request().await;
        assert_eq!((first.id.as_str(), second.id.as_str()), ("1", "2"));
        let state = MachineImageState {
            generation: demi_machines_protocol::GenerationId::parse("gen-1").unwrap(),
            base_version: BaseVersion::parse("base-1").unwrap(),
            reset_id: None,
            system_bytes: 1024.try_into().unwrap(),
            home_bytes: 2048.try_into().unwrap(),
        };
        // The later request is answered first.
        let (base, image) = match (&first.call, &second.call) {
            (MachineCall::CurrentBaseVersion(_), MachineCall::ImageState(_)) => (&first.id, &second.id),
            (MachineCall::ImageState(_), MachineCall::CurrentBaseVersion(_)) => (&second.id, &first.id),
            other => panic!("{other:?}"),
        };
        peer.ok(image, serde_json::to_value(&state).unwrap()).await;
        peer.ok(base, serde_json::json!("base-1")).await;
        let (base, image) = calls.await.unwrap();
        assert_eq!(base.unwrap().as_str(), "base-1");
        assert_eq!(image.unwrap(), Some(state));
    }

    #[tokio::test]
    async fn a_failure_reply_fails_its_call_and_the_connection_stays_usable() {
        let (_directory, path, listener) = socket();
        let (client, _deaths) = MachinesClient::new(path);
        let client = std::sync::Arc::new(client);
        let failing = {
            let client = client.clone();
            tokio::spawn(async move { client.call(HibernateParams { device_id: device("dev-9") }).await })
        };
        let mut peer = Peer::accept(&listener).await;
        let request = peer.request().await;
        let refusal = MachineResponse::Error {
            id: request.id,
            message: "no such machine".into(),
        };
        peer.write(&refusal).await;
        assert_eq!(failing.await.unwrap(), Err(MachinesError::Failed("no such machine".into())));
        // Lines that are not replies, or answer nothing asked, are dropped.
        peer.write_raw("not json\n{\"type\":\"ok\",\"id\":\"99\",\"result\":null}\n\n").await;
        let answered = {
            let client = client.clone();
            tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
        };
        let request = peer.request().await;
        peer.ok(&request.id, serde_json::json!("base-1")).await;
        assert_eq!(answered.await.unwrap().unwrap().as_str(), "base-1");
        // A result the operation does not have fails its call.
        let odd = {
            let client = client.clone();
            tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
        };
        let request = peer.request().await;
        peer.ok(&request.id, serde_json::json!(7)).await;
        assert!(matches!(odd.await.unwrap(), Err(MachinesError::Result { .. })));
    }

    #[tokio::test]
    async fn a_death_reaches_the_router_and_close_reconciles_then_disconnects_until_the_next_call() {
        let (_directory, path, listener) = socket();
        let (client, mut deaths) = MachinesClient::new(path);
        let client = std::sync::Arc::new(client);
        // A client that never connected closes without a word.
        client.close().await.unwrap();
        let first = {
            let client = client.clone();
            tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
        };
        let mut peer = Peer::accept(&listener).await;
        let request = peer.request().await;
        peer.write(&MachineResponse::Death { device_id: "dev-1".into() }).await;
        peer.ok(&request.id, serde_json::json!("base-1")).await;
        first.await.unwrap().unwrap();
        assert_eq!(deaths.recv().await.unwrap().as_str(), "dev-1");

        let closing = {
            let client = client.clone();
            tokio::spawn(async move { client.close().await })
        };
        let reconcile = peer.request().await;
        assert!(matches!(reconcile.call, MachineCall::Reconcile(_)), "{reconcile:?}");
        peer.ok(&reconcile.id, serde_json::Value::Null).await;
        closing.await.unwrap().unwrap();
        assert!(peer.closed().await);

        let again = {
            let client = client.clone();
            tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
        };
        let mut peer = Peer::accept(&listener).await;
        let request = peer.request().await;
        peer.ok(&request.id, serde_json::json!("base-2")).await;
        assert_eq!(again.await.unwrap().unwrap().as_str(), "base-2");
    }

    #[tokio::test]
    async fn a_manager_that_goes_away_fails_the_calls_in_flight_and_is_dialed_again() {
        let (directory, path, listener) = socket();
        let (client, _deaths) = MachinesClient::new(path.clone());
        let client = std::sync::Arc::new(client);
        let in_flight = {
            let client = client.clone();
            tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
        };
        let mut peer = Peer::accept(&listener).await;
        peer.request().await;
        drop(peer);
        drop(listener);
        let failed = in_flight.await.unwrap().unwrap_err();
        assert!(failed.to_string().starts_with("Machine manager unavailable during current_base_version"), "{failed}");

        // With no manager listening, a call fails at once.
        std::fs::remove_file(&path).unwrap();
        let refused = client.call(CurrentBaseVersionParams {}).await.unwrap_err();
        assert!(matches!(refused, MachinesError::Unavailable { .. }), "{refused}");

        let listener = UnixListener::bind(directory.path().join("machines.sock")).unwrap();
        let later = {
            let client = client.clone();
            tokio::spawn(async move { client.call(CurrentBaseVersionParams {}).await })
        };
        let mut peer = Peer::accept(&listener).await;
        let request = peer.request().await;
        peer.ok(&request.id, serde_json::json!("base-1")).await;
        assert_eq!(later.await.unwrap().unwrap().as_str(), "base-1");
    }
}
