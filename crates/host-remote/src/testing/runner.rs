//! A real runner for one device: the `demi-runner` the workspace built,
//! with a temporary home and state, connected to a backend end of its own
//! that serves the runner socket and the pipe routes the way the backend's
//! edge does. What reaches the connection, the adoption and the pipe claims,
//! is handed to a local task that owns the device, as the edge hands it to
//! the user's shard.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{
        Path as UrlPath, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use demi_runner_protocol::wire::{self, HelloErrorCode, Inbound, MAX_MESSAGE_BYTES, Outbound};
use demi_shell::{CommandSet, HostKey};
use futures_util::{SinkExt, StreamExt, future::ready};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Child,
    sync::{mpsc, oneshot, watch},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{CommandPolicy, TEST_DEVICE};
use crate::{
    Admission, DeviceLink, DeviceSink, DeviceSource, Link, LinkOptions, PipeRefusal, Pipes,
    RemoteHost, host_identity,
};

/// The device token the fixture's runner presents.
const TOKEN: &str = "fixture-token";

/// How long a runner may take to come online.
const ONLINE: Duration = Duration::from_secs(15);

/// The runner tests start: `DEMI_RUNNER_TEST_BINARY`, else the `demi-runner`
/// the workspace built beside the test.
pub fn runner_binary() -> PathBuf {
    std::env::var_os("DEMI_RUNNER_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| built("demi-runner"))
}

/// The runner's native fixture service, which the one test selection builds
/// (`--features demi-runner/test-fixtures`).
pub fn native_fixture_binary() -> PathBuf {
    built("demi-native-fixture")
}

/// A program Cargo built into the target directory this test runs from.
fn built(name: &str) -> PathBuf {
    let executable = std::env::current_exe().expect("the test knows its executable");
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .expect("a test runs from the target directory's deps");
    directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// What a fixture's device offers.
#[derive(Default)]
pub struct FixtureOptions {
    /// Variables of the runner's own environment: the device's.
    pub env: BTreeMap<String, String>,
    /// The commands the device's jobs call back into.
    pub commands: CommandSet,
    /// Receives a copy of every message the runner sends.
    pub tap: Option<mpsc::Sender<Outbound>>,
}

/// A real runner connected to a backend end of its own.
pub struct RunnerFixture {
    home: tempfile::TempDir,
    home_path: String,
    state: tempfile::TempDir,
    /// The backend end's address, `host:port`.
    address: String,
    env: BTreeMap<String, String>,
    device: Rc<watch::Sender<DeviceLink>>,
    pipes: Pipes,
    policy: Rc<CommandPolicy>,
    runner: Option<Child>,
    log: Rc<RefCell<String>>,
    stop: CancellationToken,
    tasks: TaskTracker,
    server: tokio::task::JoinHandle<()>,
}

/// What the edge hands the device's local task.
enum Request {
    Adopt(WebSocket),
    Source {
        id: String,
        claimed: oneshot::Sender<Result<DeviceSource, PipeRefusal>>,
    },
    Sink {
        id: String,
        claimed: oneshot::Sender<Result<DeviceSink, PipeRefusal>>,
    },
}

#[derive(Clone)]
struct Edge {
    requests: mpsc::Sender<Request>,
    token: Arc<str>,
}

impl RunnerFixture {
    /// Starts the backend end and the runner, and waits until the runner is
    /// online. It must run where local tasks can, such as a `local` test.
    pub async fn start(options: FixtureOptions) -> Self {
        let home = tempfile::tempdir().expect("a temporary home");
        let home_path = std::fs::canonicalize(home.path())
            .expect("the home has a real path")
            .to_string_lossy()
            .into_owned();
        let state = tempfile::tempdir().expect("a temporary runner state");
        write_token(state.path());
        std::fs::create_dir(state.path().join("tmp"))
            .expect("a temporary directory for the runner");
        let pipes = Pipes::new(crate::ARRIVAL);
        let policy = CommandPolicy::new(options.commands);
        let device = Rc::new(watch::Sender::new(DeviceLink::Offline { last: None }));
        let stop = CancellationToken::new();
        let tasks = TaskTracker::new();
        let (requests, incoming) = mpsc::channel(16);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port");
        let address = listener.local_addr().expect("the listener's address");
        let edge = Edge {
            requests,
            token: TOKEN.into(),
        };
        let router = Router::new()
            .route("/api/runner", get(runner_socket))
            .route("/api/pipes/{id}", get(pipe_sink).put(pipe_source))
            .with_state(edge);
        let serving = stop.clone();
        let server = tokio::spawn(async move {
            let served = axum::serve(listener, router)
                .with_graceful_shutdown(serving.cancelled_owned())
                .await;
            if let Err(error) = served {
                eprintln!("fixture server failed: {error}");
            }
        });
        tasks.spawn_local(serve_device(
            incoming,
            device.clone(),
            pipes.clone(),
            policy.clone(),
            options.tap,
            tasks.clone(),
        ));
        let log = Rc::new(RefCell::new(String::new()));
        let address = address.to_string();
        let runner = spawn_runner(
            runner_command(&home_path, state.path(), &address, &options.env),
            &log,
            &tasks,
        );
        let fixture = Self {
            home,
            home_path,
            state,
            address,
            env: options.env,
            device,
            pipes,
            policy,
            runner: Some(runner),
            log,
            stop,
            tasks,
            server,
        };
        if tokio::time::timeout(ONLINE, fixture.link()).await.is_err() {
            panic!("the runner did not come online:\n{}", fixture.log());
        }
        fixture
    }

    /// The runner's home, the default working directory of [`Self::host`].
    pub fn home(&self) -> &str {
        &self.home_path
    }

    pub fn home_dir(&self) -> &Path {
        self.home.path()
    }

    /// The device's Host, starting work in its home.
    pub fn host(&self) -> RemoteHost {
        self.host_at(&self.home_path.clone())
    }

    /// The device's Host, starting work in `cwd`.
    pub fn host_at(&self, cwd: &str) -> RemoteHost {
        RemoteHost::new(
            HostKey::new(format!("{TEST_DEVICE}:{cwd}")),
            cwd.into(),
            self.device.subscribe(),
            Admission::Free,
        )
    }

    pub fn pipes(&self) -> &Pipes {
        &self.pipes
    }

    pub fn policy(&self) -> &Rc<CommandPolicy> {
        &self.policy
    }

    /// The device's connection, once the runner is online.
    pub async fn link(&self) -> Link {
        let mut device = self.device.subscribe();
        let online = device
            .wait_for(|link| matches!(link, DeviceLink::Online(link) if !link.is_closed()))
            .await
            .expect("the device outlives its watchers");
        match &*online {
            DeviceLink::Online(link) => link.clone(),
            DeviceLink::Offline { .. } => unreachable!("waited for a connection"),
        }
    }

    /// Waits until no runner is connected.
    pub async fn offline(&self) {
        let mut device = self.device.subscribe();
        // The fixture holds the sender while anyone watches.
        let _ = device
            .wait_for(|link| matches!(link, DeviceLink::Offline { .. }))
            .await;
    }

    /// Another runner for this device, set up as the fixture's own: the
    /// same home, state and backend.
    pub fn command(&self) -> tokio::process::Command {
        runner_command(&self.home_path, self.state.path(), &self.address, &self.env)
    }

    /// What the runner printed.
    pub fn log(&self) -> String {
        self.log.borrow().clone()
    }

    /// Stops the runner, then the backend end.
    pub async fn stop(mut self) {
        if let Some(runner) = self.runner.take() {
            terminate(runner).await;
        }
        self.stop.cancel();
        if let DeviceLink::Online(link) = &*self.device.borrow() {
            link.disconnect("backend shutting down");
        }
        self.pipes.close().await;
        self.tasks.close();
        self.tasks.wait().await;
        // The server ends with its graceful shutdown.
        let _ = (&mut self.server).await;
    }
}

impl Drop for RunnerFixture {
    fn drop(&mut self) {
        // A test that failed before `stop` still ends its runner.
        if let Some(runner) = &mut self.runner {
            let _ = runner.start_kill();
        }
        self.stop.cancel();
    }
}

fn write_token(state: &Path) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(state.join("runner-token"))
        .expect("the runner token can be written");
    writeln!(file, "{TOKEN}").expect("the runner token can be written");
}

/// The runner's command line for a device: its home, its state and its
/// backend.
fn runner_command(
    home: &str,
    state: &Path,
    address: &str,
    env: &BTreeMap<String, String>,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(runner_binary());
    command
        .args(["run", "--backend", &format!("http://{address}")])
        .current_dir(home)
        .envs(env)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("DEMI_HOME", state)
        // What the runner leaves in its temporary directory goes with the
        // fixture, even when it is killed.
        .env("TMPDIR", state.join("tmp"))
        .env("DEMI_RUNNER_NAME", "fixture")
        .env_remove("DEMI_RUNNER_MANAGED")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

fn spawn_runner(
    mut command: tokio::process::Command,
    log: &Rc<RefCell<String>>,
    tasks: &TaskTracker,
) -> Child {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap_or_else(|error| {
        panic!(
            "start {}: {error}; build it with cargo build --workspace --features demi-runner/test-fixtures",
            runner_binary().display()
        )
    });
    for output in [
        child
            .stdout
            .take()
            .map(|stream| Box::new(stream) as Box<dyn tokio::io::AsyncRead + Unpin>),
        child
            .stderr
            .take()
            .map(|stream| Box::new(stream) as Box<dyn tokio::io::AsyncRead + Unpin>),
    ]
    .into_iter()
    .flatten()
    {
        let log = log.clone();
        tasks.spawn_local(async move {
            let mut lines = BufReader::new(output).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut log = log.borrow_mut();
                log.push_str(&line);
                log.push('\n');
            }
        });
    }
    child
}

/// Asks the runner to stop and waits for it, killing it after five seconds.
async fn terminate(mut runner: Child) {
    #[cfg(unix)]
    if let Some(pid) = runner
        .id()
        .and_then(|pid| rustix::process::Pid::from_raw(pid as i32))
    {
        // A runner that already exited has nothing to stop.
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    }
    if tokio::time::timeout(Duration::from_secs(5), runner.wait())
        .await
        .is_err()
    {
        // A runner that ignored the request is killed.
        let _ = runner.kill().await;
    }
}

/// The device's local task: adopts the runner's sockets and claims pipe ends
/// for the edge.
async fn serve_device(
    mut requests: mpsc::Receiver<Request>,
    device: Rc<watch::Sender<DeviceLink>>,
    pipes: Pipes,
    policy: Rc<CommandPolicy>,
    tap: Option<mpsc::Sender<Outbound>>,
    tasks: TaskTracker,
) {
    while let Some(request) = requests.recv().await {
        match request {
            Request::Adopt(socket) => {
                tasks.spawn_local(adopt(
                    socket,
                    device.clone(),
                    pipes.clone(),
                    policy.clone(),
                    tap.clone(),
                ));
            }
            Request::Source { id, claimed } => {
                // An edge request that went away takes nothing.
                let _ = claimed.send(pipes.claim_source(&id, TEST_DEVICE));
            }
            Request::Sink { id, claimed } => {
                let _ = claimed.send(pipes.claim_sink(&id, TEST_DEVICE));
            }
        }
    }
}

/// Reads the runner's hello, answers it, and serves the connection until it
/// ends.
async fn adopt(
    socket: WebSocket,
    device: Rc<watch::Sender<DeviceLink>>,
    pipes: Pipes,
    policy: Rc<CommandPolicy>,
    tap: Option<mpsc::Sender<Outbound>>,
) {
    let (mut outgoing, mut incoming) = socket.split();
    let hello = loop {
        match incoming.next().await {
            Some(Ok(Message::Binary(frame))) => break wire::decode::<Outbound>(&frame),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return,
        }
    };
    let refusal = |code: HelloErrorCode, reason: &str| Inbound::HelloError {
        code,
        reason: reason.into(),
    };
    // A connection that is still closing ends first: its end marks the
    // device offline.
    let mut watching = device.subscribe();
    let _ = watching
        .wait_for(|link| !matches!(link, DeviceLink::Online(link) if link.is_closed()))
        .await;
    let answer = match &hello {
        Ok(Outbound::Hello {
            protocol,
            device_token,
            runner,
        }) => {
            if *protocol != wire::VERSION {
                Err(refusal(
                    HelloErrorCode::UnsupportedProtocol,
                    "unsupported protocol",
                ))
            } else if device_token.as_ref().map(|token| token.expose()) != Some(TOKEN) {
                Err(refusal(HelloErrorCode::UnknownDevice, "unknown device"))
            } else if matches!(&*device.borrow(), DeviceLink::Online(link) if !link.is_closed()) {
                Err(refusal(
                    HelloErrorCode::AlreadyConnected,
                    "already connected",
                ))
            } else {
                Ok(host_identity(&runner.identity))
            }
        }
        _ => return,
    };
    let identity = match answer {
        Ok(identity) => identity,
        Err(refusal) => {
            let frame = wire::encode(&refusal)
                .expect("a valid refusal")
                .into_bytes();
            // A runner that went away hears nothing.
            let _ = outgoing.send(Message::Binary(frame.into())).await;
            return;
        }
    };
    let welcome = Inbound::HelloOk {
        device_id: TEST_DEVICE.into(),
    };
    let frame = wire::encode(&welcome)
        .expect("a valid welcome")
        .into_bytes();
    if outgoing.send(Message::Binary(frame.into())).await.is_err() {
        return;
    }
    let (link, driver) = Link::new(LinkOptions {
        device: TEST_DEVICE.into(),
        identity: identity.clone(),
        pipes,
        policy,
        ping: None,
    });
    device.send_replace(DeviceLink::Online(link.clone()));
    let incoming = incoming.filter_map(|message| {
        ready(match message {
            Ok(Message::Binary(frame)) => Some(Ok(frame.to_vec())),
            Ok(Message::Text(_)) => Some(Err("the runner sent a text frame".into())),
            Ok(_) => None,
            Err(error) => Some(Err(error.to_string())),
        })
    });
    let outgoing =
        outgoing.with(|frame: Vec<u8>| ready(Ok::<_, axum::Error>(Message::Binary(frame.into()))));
    let driver = match tap {
        Some(tap) => driver.tap(tap),
        None => driver,
    };
    driver.serve(incoming, outgoing).await;
    super::went_offline(&device, &link, identity);
}

async fn runner_socket(State(edge): State<Edge>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            // A fixture that stopped adopts nothing.
            let _ = edge.requests.send(Request::Adopt(socket)).await;
        })
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        == Some(token)
}

fn refused(refusal: PipeRefusal) -> Response {
    let status = match refusal {
        PipeRefusal::NotFound => StatusCode::NOT_FOUND,
        PipeRefusal::AlreadyConnected => StatusCode::CONFLICT,
    };
    (status, refusal.to_string()).into_response()
}

async fn pipe_source(
    State(edge): State<Edge>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if !authorized(&headers, &edge.token) {
        return (StatusCode::UNAUTHORIZED, "device token required").into_response();
    }
    let (claimed, claim) = oneshot::channel();
    if edge
        .requests
        .send(Request::Source { id, claimed })
        .await
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match claim.await {
        Ok(Ok(source)) => match source.pump(body.into_data_stream()).await {
            Ok(()) => (StatusCode::OK, "drained").into_response(),
            Err(failure) => (StatusCode::CONFLICT, failure.to_string()).into_response(),
        },
        Ok(Err(refusal)) => refused(refusal),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn pipe_sink(
    State(edge): State<Edge>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &edge.token) {
        return (StatusCode::UNAUTHORIZED, "device token required").into_response();
    }
    let (claimed, claim) = oneshot::channel();
    if edge
        .requests
        .send(Request::Sink { id, claimed })
        .await
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let mut sink = match claim.await {
        Ok(Ok(sink)) => sink,
        Ok(Err(refusal)) => return refused(refusal),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if let Err(failure) = sink.source_arrived().await {
        return (StatusCode::CONFLICT, failure.to_string()).into_response();
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(sink.into_stream()))
        .expect("a valid response")
}
