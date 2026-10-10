//! Test support (feature `testing`): a dispatcher with live execution
//! contexts on a connection that channels stand in for, so tests exercise
//! declared commands, callbacks and native calls without a backend socket.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use demi_command_protocol::{CommandContext, EditContext, Viewable};
use demi_runner_command_packages::ServiceRegistry;
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::{manifest::Manifest, wire};
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
    connection::{ConnectionHandle, Live, Reach, Relay, Request},
    job_media::{Arrival, JobMedia, MEDIA_DIRECTORY},
};

/// A dispatcher, its local endpoint and a connection owner for callbacks,
/// questions and contexts, fed by channels.
pub struct Dispatch {
    pub services: ServiceRegistry,
    pub dispatcher: Arc<Dispatcher>,
    pub server: Server,
    /// Frames the connection sends the backend.
    pub outgoing: mpsc::Receiver<wire::Frame>,
    manifest: Arc<Manifest>,
    paths: ContextPaths,
    handle: ConnectionHandle,
    /// The connection's frames to the backend.
    control: mpsc::Sender<wire::Frame>,
    closed: CancellationToken,
    /// Keeps the connection reachable while the dispatch lives.
    _reach: watch::Sender<Reach>,
    inbound: mpsc::Sender<wire::Inbound>,
    removals: mpsc::Sender<String>,
    owner: tokio::task::JoinHandle<()>,
}

impl Dispatch {
    /// Installs `manifest` (a manifest body with its `hash`) in `root`; the
    /// runner test binary stands in for the command aliases.
    pub async fn new(root: &Path, manifest: serde_json::Value, pipes: PipeClient) -> Self {
        let services =
            ServiceRegistry::new(root.join("artifacts"), None, None, root.into(), BTreeMap::new())
                .await
                .expect("service registry");
        let paths = ContextPaths::new(
            root.join("manifests"),
            std::env::current_exe().expect("test executable"),
        )
        .await
        .expect("context directory");
        let installed = contexts::install(manifest, &paths, &services.handle(), &BTreeSet::new())
            .await
            .expect("manifest installs");
        let index = watch::Sender::new(Arc::default());
        let dispatcher = Arc::new(Dispatcher {
            contexts: Contexts::new(index.subscribe()),
            services: services.handle(),
            pipes,
        });
        let server = Server::start(dispatcher.clone())
            .await
            .expect("local endpoint");
        let (control, outgoing) = mpsc::channel(32);
        let closed = CancellationToken::new();
        let reach = watch::Sender::new(Reach::Connected(Live {
            control: control.clone(),
            closed: closed.clone(),
        }));
        let (handle, requests) = ConnectionHandle::new(reach.subscribe());
        let (inbound, routed) = mpsc::channel(32);
        let (removals, removed) = mpsc::channel(16);
        let owner = tokio::spawn(serve(
            control.clone(),
            requests,
            routed,
            removed,
            index,
            closed.clone(),
        ));
        Self {
            services,
            dispatcher,
            server,
            outgoing,
            manifest: installed.manifest,
            paths,
            handle,
            control,
            closed,
            _reach: reach,
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
            directory: self
                .paths
                .directory
                .join(job_id)
                .to_string_lossy()
                .into_owned(),
            lock: self
                .paths
                .directory
                .join("edits.lock")
                .to_string_lossy()
                .into_owned(),
        };
        // The job's media are announced where its connection's frames go;
        // their lines have no job output to go to here.
        let (arrivals, mut arrived) = mpsc::unbounded_channel::<Arrival>();
        let control = self.control.clone();
        tokio::spawn(async move {
            while let Some(arrival) = arrived.recv().await {
                if let Some(medium) = arrival.medium
                    && control.send(medium).await.is_err()
                {
                    return;
                }
            }
        });
        let media = Arc::new(JobMedia::new(
            job_id.into(),
            self.paths.directory.join(job_id).join(MEDIA_DIRECTORY),
            arrivals,
        ));
        // A model that reads every medium in a tool result.
        let viewable = Viewable {
            model: "test-model".into(),
            media_types: Some([
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/webp",
                "video/mp4",
                "video/x-m4v",
                "video/quicktime",
                "video/webm",
                "application/pdf",
            ]
            .map(str::to_owned)
            .to_vec()),
        };
        let context = Arc::new(
            ExecutionContext::create(
                job_id.into(),
                command,
                self.manifest.clone(),
                edits,
                media,
                Some(viewable),
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
        self.closed.cancel();
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

/// A connection owner for callbacks, questions and contexts alone, until
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
        // Requests go before inbound messages, as in the connection's owner: a
        // call registers before it sends its request to the backend, so a
        // reply that is already queued beside the registration finds the call.
        tokio::select! {
            biased;
            _ = closed.cancelled() => return,
            request = requests.recv() => match request {
                Some(Request::Call { id, events, ended }) => relay.call(id, events, ended, &mut watches),
                Some(Request::Ask { question, abandoned }) => {
                    let frame = relay
                        .ask(question, abandoned, &mut watches)
                        .expect("question frame");
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
