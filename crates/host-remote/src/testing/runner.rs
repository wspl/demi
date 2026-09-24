//! A real runner for one device: a [`RunnerProcess`] connected to a backend
//! end of its own that serves the runner socket and the pipe routes the way
//! the backend's edge does. What reaches the connection, the adoption and
//! the pipe claims, is handed to a local task that owns the device, as the
//! edge hands it to the user's shard.

use std::{collections::BTreeMap, path::Path, rc::Rc, sync::Arc, time::Duration};

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
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::process::{RunnerProcess, RunnerProcessOptions};
use super::{CommandPolicy, TEST_DEVICE};
use crate::{
    Admission, DeviceLink, DeviceSink, DeviceSource, Link, LinkOptions, PipeRefusal, Pipes,
    RemoteHost, host_identity,
};

/// The device token the fixture's runner presents.
const TOKEN: &str = "fixture-token";

/// How long a runner may take to come online.
const ONLINE: Duration = Duration::from_secs(15);

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
    process: RunnerProcess,
    device: Rc<watch::Sender<DeviceLink>>,
    pipes: Pipes,
    policy: Rc<CommandPolicy>,
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
        let process = RunnerProcess::start(
            &format!("http://{address}"),
            RunnerProcessOptions {
                name: "fixture".into(),
                env: options.env,
                token: Some(TOKEN.into()),
                managed: false,
            },
        );
        let fixture = Self {
            process,
            device,
            pipes,
            policy,
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
        self.process.home()
    }

    pub fn home_dir(&self) -> &Path {
        self.process.home_dir()
    }

    /// The device's Host, starting work in its home.
    pub fn host(&self) -> RemoteHost {
        self.host_at(&self.home().to_owned())
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
        self.process.command()
    }

    /// What the runner printed.
    pub fn log(&self) -> String {
        self.process.output()
    }

    /// Stops the runner, then the backend end.
    pub async fn stop(mut self) {
        self.process.stop().await;
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
        // A test that failed before `stop` still ends its runner, which the
        // process ends when it is dropped, and the backend end.
        self.stop.cancel();
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
