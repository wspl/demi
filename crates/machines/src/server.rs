//! The socket server (`managed-hosts.md` § Control and ownership): one Unix
//! socket, any number of backend connections, every request handed to one
//! [`MachineService`]. Requests on a connection run concurrently and are
//! answered in completion order; a death event goes to every connection. The
//! socket's owner and group are the boundary: there is no authentication and
//! no TCP listener.

use std::{
    collections::HashMap,
    future::Future,
    io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    rc::Rc,
};

use demi_machines_protocol::{
    DeviceId, MAX_LINE_BYTES, MachineCall, MachineRequest, MachineResponse, decode_request,
    encode_line,
};
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
/// as they come, so a full queue only delays the requests that wait on it.
const OUTBOX_LINES: usize = 64;

/// The pause after a failed accept.
const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// What serves the socket's requests.
pub trait MachineService {
    type Error: std::error::Error;

    /// Runs one call and returns its result as the `ok` reply carries it.
    fn handle(&self, call: MachineCall) -> impl Future<Output = Result<serde_json::Value, Self::Error>>;
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
                io::Error::new(error.kind(), format!("failed to listen on {}: {error}", path.display()))
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
    shutdown: impl Future<Output = ()>,
) -> InFlight {
    let requests = TaskTracker::new();
    let mut connections = JoinSet::new();
    let mut outboxes: HashMap<u64, mpsc::WeakSender<Vec<u8>>> = HashMap::new();
    let mut next_connection = 0_u64;
    let mut deaths_open = true;
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
                    outboxes.insert(id, outbox.downgrade());
                    let connection = connection(stream, service.clone(), outbox, queued, requests.clone());
                    connections.spawn_local(async move {
                        connection.await;
                        id
                    });
                }
                Err(error) => {
                    tracing::warn!("machines: accepting a connection failed: {error}");
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
        }
    }
    connections.shutdown().await;
    let Socket { listener, path } = socket;
    drop(listener);
    let removed = blocking::run(move |_| fs_err::remove_file(path)).await;
    if let Err(error) = removed {
        tracing::warn!("machines: removing the socket failed: {error}");
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
                tracing::debug!("machines: connection error: {error}");
                return;
            }
            Err(error) => {
                tracing::warn!("machines: dropping a connection over a bad frame: {error}");
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
                tracing::warn!("machines: dropping a connection over a bad frame: {error}");
                return;
            }
        }
    }
}

async fn write_lines(mut write: OwnedWriteHalf, mut queued: mpsc::Receiver<Vec<u8>>) {
    while let Some(line) = queued.recv().await {
        if let Err(error) = write.write_all(&line).await {
            tracing::debug!("machines: connection error: {error}");
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
    let response = match service.handle(request.call).await {
        Ok(result) => MachineResponse::Ok { id: request.id, result },
        Err(error) => {
            tracing::warn!("machines: {operation} failed: {}", chain(&error));
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

#[cfg(test)]
mod tests {
    use std::{cell::Cell, cell::RefCell, time::Duration};

    use demi_machines_protocol::decode_response;
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        sync::oneshot,
    };

    use super::*;

    /// Records each call and answers it from a script: an operation named in
    /// `failures` fails once with that message, and `current_base_version`
    /// waits for `release` when one is set.
    #[derive(Default)]
    struct ScriptedService {
        calls: RefCell<Vec<MachineCall>>,
        finished: Cell<usize>,
        failures: RefCell<Vec<(&'static str, &'static str)>>,
        release: RefCell<Option<oneshot::Receiver<()>>>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    struct Failed(&'static str, #[source] io::Error);

    impl ScriptedService {
        fn result(call: &MachineCall) -> serde_json::Value {
            match call {
                MachineCall::CurrentBaseVersion(_) => serde_json::json!("b".repeat(64)),
                MachineCall::RuntimeState(_) => serde_json::json!("stopped"),
                _ => serde_json::Value::Null,
            }
        }
    }

    impl MachineService for ScriptedService {
        type Error = Failed;

        async fn handle(&self, call: MachineCall) -> Result<serde_json::Value, Failed> {
            self.calls.borrow_mut().push(call.clone());
            let failure = {
                let mut failures = self.failures.borrow_mut();
                let position = failures.iter().position(|(operation, _)| *operation == call.name());
                position.map(|position| failures.remove(position).1)
            };
            if let Some(message) = failure {
                return Err(Failed(message, io::Error::other("the cause")));
            }
            if let MachineCall::CurrentBaseVersion(_) = call {
                let release = self.release.borrow_mut().take();
                if let Some(release) = release {
                    // A dropped sender ends the wait as well.
                    let _ = release.await;
                }
            }
            self.finished.set(self.finished.get() + 1);
            Ok(Self::result(&call))
        }
    }

    struct Harness {
        directory: tempfile::TempDir,
        service: Rc<ScriptedService>,
        deaths: mpsc::Sender<DeviceId>,
        stop: oneshot::Sender<()>,
        server: tokio::task::JoinHandle<InFlight>,
    }

    impl Harness {
        async fn start() -> Self {
            let directory = tempfile::tempdir().expect("temporary directory");
            let socket = Socket::bind(&directory.path().join("sock/machines.sock"))
                .await
                .expect("bind");
            let service = Rc::new(ScriptedService::default());
            let (deaths, received) = mpsc::channel(4);
            let (stop, stopped) = oneshot::channel::<()>();
            let server = tokio::task::spawn_local(serve(socket, service.clone(), received, async {
                // A dropped sender stops the server as well.
                let _ = stopped.await;
            }));
            Self {
                directory,
                service,
                deaths,
                stop,
                server,
            }
        }

        async fn connect(&self) -> Client {
            let path = self.directory.path().join("sock/machines.sock");
            let stream = UnixStream::connect(path).await.expect("connect");
            let (read, write) = stream.into_split();
            Client {
                lines: BufReader::new(read).lines(),
                write,
            }
        }

        /// Stops the server and waits for the requests it left running.
        async fn stop(self) -> Rc<ScriptedService> {
            // The server may have stopped already when the test dropped it.
            let _ = self.stop.send(());
            self.server.await.expect("server").wait().await;
            self.service
        }
    }

    struct Client {
        lines: tokio::io::Lines<BufReader<OwnedReadHalf>>,
        write: OwnedWriteHalf,
    }

    impl Client {
        async fn send(&mut self, bytes: &[u8]) {
            self.write.write_all(bytes).await.expect("write");
        }

        /// The next message, or `None` once the server closed the connection.
        /// Linux reports a close with unread data as a reset.
        async fn receive(&mut self) -> Option<MachineResponse> {
            let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
                .await
                .expect("a line in time")
                .ok()??;
            Some(decode_response(&line).expect("a response"))
        }
    }

    #[tokio::test(flavor = "local")]
    async fn every_call_reaches_the_service_and_its_result_the_client() {
        let harness = Harness::start().await;
        let mut client = harness.connect().await;
        let lines = [
            r#"{"id":"1","op":"reconcile","params":{}}"#,
            r#"{"id":"2","op":"current_base_version","params":{}}"#,
            r#"{"id":"3","op":"image_state","params":{"deviceId":"dev-1"}}"#,
            r#"{"id":"4","op":"runtime_state","params":{"deviceId":"dev-1"}}"#,
            r#"{"id":"5","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"http://backend","deviceToken":"tok"}}}"#,
            r#"{"id":"6","op":"checkpoint","params":{"deviceId":"dev-1"}}"#,
            r#"{"id":"7","op":"grow_volume","params":{"deviceId":"dev-1","volume":"home","bytes":4096}}"#,
            r#"{"id":"8","op":"reset","params":{"deviceId":"dev-1","operationId":"op-1","baseVersion":"base-2"}}"#,
            r#"{"id":"9","op":"hibernate","params":{"deviceId":"dev-1"}}"#,
        ];
        for line in lines {
            let request = decode_request(line).expect("request");
            client.send(format!("{line}\n").as_bytes()).await;
            assert_eq!(
                client.receive().await,
                Some(MachineResponse::Ok {
                    id: request.id,
                    result: ScriptedService::result(&request.call),
                })
            );
        }
        let service = harness.stop().await;
        let expected: Vec<_> = lines
            .iter()
            .map(|line| decode_request(line).expect("request").call)
            .collect();
        assert_eq!(*service.calls.borrow(), expected);
    }

    #[tokio::test(flavor = "local")]
    async fn a_failure_is_an_error_reply_and_the_connection_stays_usable() {
        let harness = Harness::start().await;
        harness.service.failures.borrow_mut().push(("hibernate", "no such machine"));
        let mut client = harness.connect().await;
        client
            .send(b"{\"id\":\"1\",\"op\":\"hibernate\",\"params\":{\"deviceId\":\"dev-9\"}}\n")
            .await;
        assert_eq!(
            client.receive().await,
            Some(MachineResponse::Error {
                id: "1".into(),
                message: "no such machine".into()
            })
        );
        // An empty line is skipped.
        client.send(b"\n{\"id\":\"2\",\"op\":\"reconcile\",\"params\":{}}\n").await;
        assert!(matches!(client.receive().await, Some(MachineResponse::Ok { id, .. }) if id == "2"));
        harness.stop().await;
    }

    #[tokio::test(flavor = "local")]
    async fn concurrent_requests_are_answered_as_they_finish() {
        let harness = Harness::start().await;
        let (release, wait) = oneshot::channel();
        *harness.service.release.borrow_mut() = Some(wait);
        let mut client = harness.connect().await;
        client
            .send(b"{\"id\":\"slow\",\"op\":\"current_base_version\",\"params\":{}}\n{\"id\":\"fast\",\"op\":\"reconcile\",\"params\":{}}\n")
            .await;
        assert!(matches!(client.receive().await, Some(MachineResponse::Ok { id, .. }) if id == "fast"));
        release.send(()).expect("the slow request waits");
        assert!(matches!(client.receive().await, Some(MachineResponse::Ok { id, .. }) if id == "slow"));
        harness.stop().await;
    }

    #[tokio::test(flavor = "local")]
    async fn a_death_reaches_every_connection() {
        let harness = Harness::start().await;
        let mut first = harness.connect().await;
        let mut second = harness.connect().await;
        // A round trip on each shows the server has accepted both.
        for client in [&mut first, &mut second] {
            client.send(b"{\"id\":\"1\",\"op\":\"reconcile\",\"params\":{}}\n").await;
            client.receive().await.expect("reply");
        }
        let device = DeviceId::parse("dev-1").expect("device id");
        harness.deaths.send(device).await.expect("server");
        for client in [&mut first, &mut second] {
            assert_eq!(
                client.receive().await,
                Some(MachineResponse::Death {
                    device_id: "dev-1".into()
                })
            );
        }
        harness.stop().await;
    }

    #[tokio::test(flavor = "local")]
    async fn a_bad_line_drops_its_connection_before_the_lines_behind_it() {
        let harness = Harness::start().await;
        let next = b"{\"id\":\"2\",\"op\":\"reconcile\",\"params\":{}}\n";
        for bad in [
            b"{\"id\":\"1\",\"op\":\"wake\",\"params\":{}}\n".to_vec(),
            b"not json\n".to_vec(),
            vec![0xff, b'\n'],
            [vec![b'x'; MAX_LINE_BYTES + 1], vec![b'\n']].concat(),
        ] {
            let mut client = harness.connect().await;
            // The server may close before it has read everything.
            let _ = client.write.write_all(&[bad, next.to_vec()].concat()).await;
            assert_eq!(client.receive().await, None);
        }
        let service = harness.stop().await;
        assert!(service.calls.borrow().is_empty());
    }

    #[tokio::test(flavor = "local")]
    async fn a_reply_for_a_closed_connection_is_discarded_and_its_operation_completes() {
        let harness = Harness::start().await;
        let (release, wait) = oneshot::channel();
        *harness.service.release.borrow_mut() = Some(wait);
        let mut client = harness.connect().await;
        client
            .send(b"{\"id\":\"1\",\"op\":\"current_base_version\",\"params\":{}}\n")
            .await;
        while harness.service.calls.borrow().is_empty() {
            tokio::task::yield_now().await;
        }
        drop(client);
        release.send(()).expect("the request waits");
        let service = harness.stop().await;
        assert_eq!(service.finished.get(), 1);
    }

    #[tokio::test(flavor = "local")]
    async fn the_socket_replaces_a_stale_socket_refuses_other_files_and_is_removed_at_stop() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("machines.sock");
        drop(std::os::unix::net::UnixListener::bind(&path).expect("stale socket"));
        let socket = Socket::bind(&path).await.expect("replaces a stale socket");
        let mode = std::fs::metadata(&path).expect("socket").permissions().mode();
        assert_eq!(mode & 0o777, 0o660);
        let (_deaths, received) = mpsc::channel(1);
        let requests = serve(socket, Rc::new(ScriptedService::default()), received, async {}).await;
        requests.wait().await;
        assert!(!path.exists());
        std::fs::write(&path, "data").expect("a file");
        assert!(Socket::bind(&path).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("kept"), "data");
    }
}
