//! Test support: a dispatcher with live execution contexts on a connection
//! that channels stand in for, so tests exercise declared commands, callbacks
//! and native calls without a backend socket.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use demi_command_service::protocol::{CommandContext, EditContext};
use demi_runner_protocol::values::{BackendUrl, DeviceToken};
use demi_runner_protocol::manifest::Manifest;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::{
    commands::{
        contexts::{self, ContextPaths, ContextTable, Contexts, ExecutionContext},
        dispatch::Dispatcher,
        local::Server,
    },
    connection::{ConnectionHandle, Relay, Request, wire},
    management::Management,
    pipes::PipeClient,
    services::ServiceRegistry,
};

/// A dispatcher, its local endpoint and a connection owner for callbacks,
/// artifact locations and contexts, fed by channels.
pub struct Dispatch {
    pub services: ServiceRegistry,
    pub dispatcher: Arc<Dispatcher>,
    pub server: Server,
    /// Frames the connection sends the backend.
    pub outgoing: mpsc::Receiver<wire::Frame>,
    manifest: Arc<Manifest>,
    paths: ContextPaths,
    handle: ConnectionHandle,
    inbound: mpsc::Sender<wire::Inbound>,
    removals: mpsc::Sender<String>,
    owner: tokio::task::JoinHandle<()>,
}

impl Dispatch {
    /// Installs `manifest` (a manifest body with its `hash`) in `root`; the
    /// runner test binary stands in for the command aliases.
    pub async fn new(root: &Path, manifest: serde_json::Value, pipes: PipeClient) -> Self {
        let services = ServiceRegistry::new(root.join("artifacts"), root.into(), BTreeMap::new())
            .await
            .expect("service registry");
        let paths = ContextPaths::new(
            root.join("manifests"),
            std::env::current_exe().expect("test executable"),
        )
        .await
        .expect("context directory");
        let installed = contexts::install(manifest, &paths, &services.handle())
            .await
            .expect("manifest installs");
        let index = watch::Sender::new(Arc::default());
        let dispatcher = Arc::new(Dispatcher {
            contexts: Contexts::new(index.subscribe()),
            services: services.handle(),
            pipes,
            management: Management::new("a".repeat(32), "test".into(), CancellationToken::new()),
        });
        let server = Server::start(dispatcher.clone()).await.expect("local endpoint");
        let (control, outgoing) = mpsc::channel(32);
        let closed = CancellationToken::new();
        let (handle, requests) = ConnectionHandle::new(control.clone(), closed);
        let (inbound, routed) = mpsc::channel(32);
        let (removals, removed) = mpsc::channel(16);
        let owner = tokio::spawn(serve(
            control,
            requests,
            routed,
            removed,
            index,
            handle.closed().clone(),
        ));
        Self {
            services,
            dispatcher,
            server,
            outgoing,
            manifest: installed.manifest,
            paths,
            handle,
            inbound,
            removals,
            owner,
        }
    }

    /// A live context for `job_id`, which ends when the guard drops.
    pub async fn context(
        &self,
        job_id: &str,
        command: CommandContext,
    ) -> (Arc<ExecutionContext>, ContextGuard) {
        let edits = EditContext {
            directory: self.paths.directory.join(job_id).to_string_lossy().into_owned(),
            lock: self.paths.directory.join("edits.lock").to_string_lossy().into_owned(),
        };
        let context = Arc::new(
            ExecutionContext::create(
                job_id.into(),
                command,
                self.manifest.clone(),
                edits,
                self.handle.clone(),
                &self.paths,
            )
            .await
            .expect("execution context"),
        );
        let leases = contexts::leases(&context.manifest, &self.services.handle()).await;
        self.handle
            .register_context(context.clone(), leases)
            .await
            .expect("context registers");
        (
            context,
            ContextGuard {
                job_id: job_id.into(),
                removals: self.removals.clone(),
            },
        )
    }

    /// Hands the connection a message from the backend.
    pub async fn deliver(&self, message: wire::Inbound) {
        self.inbound.send(message).await.expect("connection owner");
    }

    pub async fn close(self) {
        self.server.close().await.expect("local endpoint closes");
        self.handle.closed().cancel();
        self.owner.await.expect("connection owner");
        self.services.close().await;
    }
}

/// Ends its context when dropped.
pub struct ContextGuard {
    job_id: String,
    removals: mpsc::Sender<String>,
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        // A test's few contexts fit the queue; a closed owner has none left.
        let _ = self.removals.try_send(std::mem::take(&mut self.job_id));
    }
}

/// A connection owner for callbacks, locations and contexts alone, until
/// `closed`.
async fn serve(
    control: mpsc::Sender<wire::Frame>,
    mut requests: mpsc::Receiver<Request>,
    mut inbound: mpsc::Receiver<wire::Inbound>,
    mut removals: mpsc::Receiver<String>,
    index: watch::Sender<Arc<contexts::ContextIndex>>,
    closed: CancellationToken,
) {
    let mut relay = Relay::default();
    let mut contexts = ContextTable::new(index);
    let mut watches = JoinSet::new();
    loop {
        tokio::select! {
            _ = closed.cancelled() => return,
            request = requests.recv() => match request {
                Some(Request::Call { id, events, ended }) => relay.call(id, events, ended, &mut watches),
                Some(Request::Locate { owner, sha256, reply, abandoned }) => {
                    let frame = relay
                        .locate(owner, sha256, reply, abandoned, &mut watches)
                        .expect("location request frame");
                    let _closed = control.send(frame).await;
                }
                Some(Request::Context { context, leases, reply }) => {
                    let _left = reply.send(contexts.insert(context, leases));
                }
                None => return,
            },
            Some(message) = inbound.recv() => {
                relay.route(&message);
            }
            Some(job_id) = removals.recv() => contexts.remove(&job_id),
            Some(ended) = watches.join_next() => {
                relay.ended(ended.expect("relay watches do not panic"));
            }
        }
    }
}

/// A Host as a backend sees it: a registration's parts and one connection
/// owner, with channels for the backend's side of the connection.
pub struct Host {
    pub root: std::path::PathBuf,
    backend: mpsc::Sender<wire::Inbound>,
    frames: mpsc::Receiver<wire::Frame>,
    connection: tokio::task::JoinHandle<std::io::Result<crate::connection::End>>,
    registry: ServiceRegistry,
    server: Server,
    log: crate::host_log::HostLogWriter,
    stop: CancellationToken,
}

impl Host {
    /// Starts a connection in `root` whose jobs start from `env`, and takes
    /// its `hello`.
    pub async fn start(root: &Path, env: BTreeMap<String, String>) -> Self {
        let stop = CancellationToken::new();
        let state = crate::state::RunnerState::open(root.join("state"))
            .await
            .expect("runner state");
        let registry = ServiceRegistry::new(root.join("artifacts"), root.into(), env.clone())
            .await
            .expect("service registry");
        let (log, _layer) = crate::host_log::open(root.join("log")).await.expect("host log");
        let backend: BackendUrl = "http://127.0.0.1:1".parse().expect("backend URL");
        let token = watch::Sender::new(Some(
            DeviceToken::try_from("test-token".to_owned()).expect("device token"),
        ));
        let pipes = PipeClient::new(&backend, token.subscribe()).expect("pipe client");
        let index = watch::Sender::new(Arc::default());
        let management = Management::new("a".repeat(32), "test".into(), stop.clone());
        let dispatcher = Arc::new(Dispatcher {
            contexts: Contexts::new(index.subscribe()),
            services: registry.handle(),
            pipes: pipes.clone(),
            management: management.clone(),
        });
        let server = Server::start(dispatcher.clone()).await.expect("local endpoint");
        let registered = crate::connection::Registered {
            backend,
            runner: wire::RunnerInfo {
                native_target: Some(crate::services::target().into()),
                name: "test".into(),
                platform: "test".into(),
                version: "test".into(),
                managed: None,
                identity: wire::HostIdentity {
                    uid: 1000,
                    gid: 1000,
                    hostname: "test".into(),
                    home_dir: root.to_string_lossy().into_owned(),
                },
            },
            state: Arc::new(state),
            token,
            management,
            services: registry.handle(),
            dispatcher,
            index,
            paths: ContextPaths::new(
                root.join("commands"),
                std::env::current_exe().expect("test executable"),
            )
            .await
            .expect("context directory"),
            pipes,
            log: log.reader(),
            shell: crate::shell::ShellRuntime::current(),
            endpoint: server.endpoint().into(),
            cwd: root.into(),
            env,
            volumes: Vec::new(),
        };
        let (transport, backend, mut frames) = crate::connection::Transport::channels();
        let connection = tokio::spawn(async move {
            crate::connection::serve(&registered, transport).await
        });
        let hello = wire::decode(&frames.recv().await.expect("hello").into_bytes());
        assert!(matches!(hello, Ok(wire::Outbound::Hello { .. })), "{hello:?}");
        Self {
            root: root.into(),
            backend,
            frames,
            connection,
            registry,
            server,
            log,
            stop,
        }
    }

    /// Accepts the registration, as the backend's `hello_ok` does.
    pub async fn online(self) -> Self {
        self.send(wire::Inbound::HelloOk {
            device_id: "device".into(),
        })
        .await;
        self
    }

    pub async fn send(&self, message: wire::Inbound) {
        self.backend.send(message).await.expect("connection owner");
    }

    /// The next message the Host sends.
    pub async fn frame(&mut self) -> wire::Outbound {
        let frame = self.frames.recv().await.expect("a frame");
        wire::decode(&frame.into_bytes()).expect("the Host sends valid messages")
    }

    /// Ends the connection as the runner's stop does, and everything else.
    pub async fn close(self) {
        self.stop.cancel();
        self.connection
            .await
            .expect("connection owner")
            .expect("connection ends cleanly");
        self.server.close().await.expect("local endpoint closes");
        self.registry.close().await;
        self.log.close().await;
    }
}
