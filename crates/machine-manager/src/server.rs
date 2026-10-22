//! The socket server (`crates/machine-manager-protocol/src/lib.rs`): one Unix
//! socket, any number of backend connections, every request handed to one
//! [`MachineService`].
//!
//! - Requests on a connection run concurrently and are answered in completion
//!   order.
//! - A death event and a line of the manager's log go to every connection.
//!
//! The socket's owner and group are the boundary: there is no authentication
//! and no TCP listener. The group includes only trusted backend infrastructure,
//! and the backend never runs as root merely to reach the socket.

use std::{
    collections::HashMap,
    future::Future,
    io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    rc::Rc,
};

use demi_machine_manager_protocol::{
    DeviceId, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, decode_request,
    WIRE_VERSION, encode_line,
};
use demi_shared_types::LogLine;
use futures_util::StreamExt;
use tokio::{
    io::AsyncWriteExt,
    net::{
        UnixListener, UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::mpsc,
    task::JoinSet,
};
use tokio_util::{
    codec::{FramedRead, LinesCodec, LinesCodecError},
    task::TaskTracker,
};

use crate::blocking;

/// Lines waiting for one connection's socket. The backend reads its replies
/// as they come, so a full queue only delays the requests that wait on it; an
/// unbounded one would grow memory for a connection that stopped reading.
const OUTBOX_LINES: usize = 64;

/// The pause after a failed accept.
const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// What serves the socket's requests.
pub trait MachineService {
    type Error: std::error::Error;

    /// Runs one call and returns its result as the `ok` reply carries it.
    fn handle(
        &self,
        call: MachineCall,
    ) -> impl Future<Output = Result<serde_json::Value, Self::Error>>;
}

/// The bound socket; [`serve`] removes its file when it stops.
pub struct Socket {
    listener: UnixListener,
    path: PathBuf,
}

impl Socket {
    /// Binds `path` with mode 0660, so the service's group may connect. Its
    /// parent is created; a socket an earlier process left there is replaced,
    /// and any other file is refused rather than deleted.
    pub async fn bind(path: &Path) -> io::Result<Self> {
        let path = path.to_owned();
        let (listener, path) = blocking::run(move |_| {
            if let Some(parent) = path.parent() {
                fs_err::create_dir_all(parent)?;
            }
            match fs_err::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_socket() => fs_err::remove_file(&path)?,
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{} exists and is not a socket", path.display()),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let listener = std::os::unix::net::UnixListener::bind(&path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("failed to listen on {}: {error}", path.display()),
                )
            })?;
            fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
            listener.set_nonblocking(true)?;
            Ok((listener, path))
        })
        .await?;
        Ok(Self {
            listener: UnixListener::from_std(listener)?,
            path,
        })
    }
}

/// Requests still running after the server stopped. Their operations
/// complete, and their replies are discarded.
pub struct InFlight(TaskTracker);

impl InFlight {
    /// Waits for every request to finish.
    pub async fn wait(self) {
        self.0.wait().await;
    }
}

/// Serves `socket` until `shutdown` completes, then stops accepting, drops
/// every connection and removes the socket file. Requests still running are
/// returned: the caller waits for them once the service no longer admits
/// work, so none of them is cut short.
pub async fn serve<S: MachineService + 'static>(
    socket: Socket,
    service: Rc<S>,
    mut deaths: mpsc::Receiver<DeviceId>,
    mut lines: mpsc::Receiver<LogLine>,
    shutdown: impl Future<Output = ()>,
) -> InFlight {
    let requests = TaskTracker::new();
    let mut connections = JoinSet::new();
    let mut outboxes: HashMap<u64, mpsc::WeakSender<Vec<u8>>> = HashMap::new();
    let mut next_connection = 0_u64;
    let mut deaths_open = true;
    let mut lines_open = true;
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => break,
            accepted = socket.listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let id = next_connection;
                    next_connection += 1;
                    let (outbox, queued) = mpsc::channel(OUTBOX_LINES);
                    // The connection's first line names the wire's version;
                    // its queue is new, so it has room.
                    let hello = encode_line(&MachineResponse::Hello {
                        version: WIRE_VERSION,
                    });
                    outbox.try_send(hello).expect("a new connection's queue has room");
                    outboxes.insert(id, outbox.downgrade());
                    let connection = connection(stream, service.clone(), outbox, queued, requests.clone());
                    connections.spawn_local(async move {
                        connection.await;
                        id
                    });
                }
                Err(error) => {
                    tracing::warn!(event = "manager.accept_failed", %error, "accepting a connection failed");
                    // A failure such as a lack of open files lasts a while;
                    // retrying at once would only spin.
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
            Some(ended) = connections.join_next() => {
                if let Ok(id) = ended {
                    outboxes.remove(&id);
                }
            }
            death = deaths.recv(), if deaths_open => match death {
                Some(device) => {
                    let line = encode_line(&MachineResponse::Death { device_id: device.to_string() });
                    for outbox in outboxes.values() {
                        let outbox = outbox.clone();
                        let line = line.clone();
                        // Each connection gets the event through its own queue, so
                        // a connection that lags delays only itself.
                        requests.spawn_local(async move {
                            if let Some(outbox) = outbox.upgrade() {
                                // An error means the connection is gone.
                                let _ = outbox.send(line).await;
                            }
                        });
                    }
                }
                None => deaths_open = false,
            },
            line = lines.recv(), if lines_open => match line {
                Some(line) => {
                    let line = encode_line(&MachineResponse::Log { line });
                    // A reader holds a line whole up to the wire's limit, and
                    // past it drops its connection, which a line of the log
                    // is not worth.
                    if line.len() <= MAX_LINE_BYTES {
                        for outbox in outboxes.values().filter_map(mpsc::WeakSender::upgrade) {
                            // A connection whose queue is full stays behind:
                            // the replies and deaths in it matter more, and
                            // the journal has the line.
                            let _ = outbox.try_send(line.clone());
                        }
                    }
                }
                None => lines_open = false,
            },
        }
    }
    connections.shutdown().await;
    let Socket { listener, path } = socket;
    drop(listener);
    let removed = blocking::run(move |_| fs_err::remove_file(path)).await;
    if let Err(error) = removed {
        tracing::warn!(event = "manager.socket_remove_failed", %error, "removing the socket failed");
    }
    requests.close();
    InFlight(requests)
}

/// Serves one connection: reads its requests until a bad line or its end,
/// and writes its replies. A bad line ends the connection; the lines behind
/// it are not served.
async fn connection<S: MachineService + 'static>(
    stream: UnixStream,
    service: Rc<S>,
    outbox: mpsc::Sender<Vec<u8>>,
    queued: mpsc::Receiver<Vec<u8>>,
    requests: TaskTracker,
) {
    let (read, write) = stream.into_split();
    let reading = read_requests(read, service, outbox, requests);
    let writing = write_lines(write, queued);
    tokio::pin!(reading, writing);
    tokio::select! {
        // The reader dropped its sender: the writer sends what is queued and
        // ends once no request can add to it.
        () = &mut reading => writing.await,
        // The socket broke; there is no one left to answer.
        () = &mut writing => {}
    }
}

async fn read_requests<S: MachineService + 'static>(
    read: OwnedReadHalf,
    service: Rc<S>,
    outbox: mpsc::Sender<Vec<u8>>,
    requests: TaskTracker,
) {
    let mut lines = FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE_BYTES));
    while let Some(line) = lines.next().await {
        let line = match line {
            Ok(line) => line,
            Err(LinesCodecError::Io(error)) if error.kind() != io::ErrorKind::InvalidData => {
                tracing::debug!(event = "manager_connection.errored", %error, "a connection failed");
                return;
            }
            Err(error) => {
                tracing::warn!(event = "manager_connection.dropped", reason = "bad_frame", %error, "a connection was dropped over a bad frame");
                return;
            }
        };
        if line.is_empty() {
            continue;
        }
        match decode_request(&line) {
            Ok(request) => {
                requests.spawn_local(answer(service.clone(), request, outbox.downgrade()));
            }
            Err(error) => {
                tracing::warn!(event = "manager_connection.dropped", reason = "bad_frame", %error, "a connection was dropped over a bad frame");
                return;
            }
        }
    }
}

async fn write_lines(mut write: OwnedWriteHalf, mut queued: mpsc::Receiver<Vec<u8>>) {
    while let Some(line) = queued.recv().await {
        if let Err(error) = write.write_all(&line).await {
            tracing::debug!(event = "manager_connection.errored", %error, "a connection failed");
            return;
        }
    }
}

/// Runs one request and queues its reply, unless its connection has closed:
/// the operation completes either way.
async fn answer<S: MachineService>(
    service: Rc<S>,
    request: MachineRequest,
    outbox: mpsc::WeakSender<Vec<u8>>,
) {
    let operation = request.call.name();
    let started = std::time::Instant::now();
    let response = match service.handle(request.call).await {
        Ok(result) => {
            tracing::info!(
                event = "manager_call.ended",
                operation,
                outcome = "ok",
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "a request ended"
            );
            MachineResponse::Ok {
                id: request.id,
                result,
            }
        }
        Err(error) => {
            tracing::warn!(
                event = "manager_call.ended",
                operation,
                outcome = "failed",
                error_chain = %chain(&error),
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "a request ended"
            );
            MachineResponse::Error {
                id: request.id,
                message: error.to_string(),
            }
        }
    };
    if let Some(outbox) = outbox.upgrade() {
        // An error means the connection is gone.
        let _ = outbox.send(encode_line(&response)).await;
    }
}

/// An error and its causes, for the log: the reply carries only the first.
pub fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
