//! The connection's owner: it reads each message and routes it, and it owns
//! what lasts as long as the connection.
//!
//! The jobs outlive it: it serves what the registration keeps while it lasts
//! ([`Kept`]). Every branch of its loop returns without waiting for the work it
//! started, apart from the wait for room in the queue of a stream to the
//! backend, which the link drains on its own. A command's live input goes to a
//! task of its own, so a command that reads none holds nothing here.

use demi_shared_link::{Held, Outgoing};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use demi_runner_command_packages::ServiceHandle;
use demi_runner_direct::{Addresses, Direct};
use demi_runner_host::{
    files,
    host::HostServer,
    volumes::Volumes,
};
use demi_runner_jobs::{
    commands::{
        contexts::{ContextIndex, ContextPaths, Installed, Manifests},
        dispatch::Dispatcher,
        streams::ServiceStreams,
    },
    connection::Live,
    kept_output::KeptReader,
    scratch::Scratch,
    tasks::{InputSender, TaskCommand, TaskSpec, WorkId, failure_exit},
};
use demi_runner_process::{
    backend::Backend,
    job_shell::JobShell,
    pipes::{PipeClient, report_pipe},
};
use demi_runner_protocol::{
    values::DeviceToken,
    wire::{self, Inbound},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use super::{
    Transport,
    kept::Kept,
};
use crate::{
    direct::HostOperations,
    host_log::{self, HostLogReader},
    management::{Management, Phase, Revocation},
    state::{RunnerConfig, RunnerState},
    update,
};

/// How a connection ended.
pub enum End {
    Stopped,
    Disconnected,
    /// The backend refused the hello with this code.
    Rejected(wire::HelloErrorCode),
    /// The backend revoked the device; the projects of these names went
    /// with it.
    Revoked(Vec<String>),
}

/// What each connection takes from its registration.
pub struct Registered {
    pub backend: Backend,
    /// The installation an installer made, which downloads its backend's
    /// next runner release ahead and updates itself to it; none for a
    /// runner started otherwise.
    pub installed: Option<Arc<update::Installed>>,
    pub runner: wire::RunnerInfo,
    pub state: Arc<RunnerState>,
    pub token: watch::Sender<Option<DeviceToken>>,
    pub management: Arc<Management>,
    pub services: ServiceHandle,
    /// What the artifact cache holds, which each connection reports
    /// (`crates/runner-command-packages/src/cache.rs`).
    pub cached: watch::Receiver<Vec<wire::HostArtifact>>,
    pub dispatcher: Arc<Dispatcher>,
    /// Where the current connection publishes its live contexts.
    pub index: watch::Sender<Arc<ContextIndex>>,
    pub paths: ContextPaths,
    pub pipes: PipeClient,
    pub log: HostLogReader,
    /// The job root (`crates/runner-jobs/src/job_directories.rs`).
    pub jobs: PathBuf,
    /// Where each conversation has its scratch directory.
    pub scratch: Scratch,
    pub shell: Arc<dyn JobShell>,
    /// The names no declared root command may take: the runner's own and
    /// its shell's builtins.
    pub reserved: BTreeSet<String>,
    /// The local endpoint command clients reach.
    pub endpoint: String,
    /// The default working directory and the environment jobs start from.
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    /// A managed guest's writable filesystems, which a `sync` flushes;
    /// none on a paired device.
    pub volumes: Vec<PathBuf>,
    /// The command that removes this runner; none for a managed guest's.
    pub removal: Option<String>,
    /// The number this start of the runner drew, which each hello names.
    pub instance: u64,
}

impl Registered {
    /// Tells the person at the console, once per process, that the device
    /// is paired as `name` and how to remove the runner again; an installer
    /// shows them these lines (`crates/runner/src/console.rs`). A managed guest's runner, which nobody pairs, says nothing.
    fn announce_paired(&self, name: &str) {
        if let Some(removal) = &self.removal {
            self.management.tell_paired(name, removal);
        }
    }
}

/// What the owner's child tasks hand back.
enum Work {
    Installed {
        installation: u64,
        hash: String,
        result: io::Result<Installed>,
    },
    Stored(io::Result<()>),
    /// The task that fed the live input of this command ended.
    Fed(String),
    /// The claimed device token was stored for the device of this name.
    Paired {
        result: io::Result<()>,
        name: String,
    },
    Done,
}

struct Owner<'r> {
    registered: &'r Registered,
    /// What the registration keeps, which this connection serves.
    kept: &'r mut Kept,
    /// This connection's messages to the backend and its end.
    live: Live,
    work: JoinSet<Work>,
    host: HostServer,
    streams: ServiceStreams,
    volumes: Volumes,
    /// What the artifact cache holds.
    cached: watch::Receiver<Vec<wire::HostArtifact>>,
    /// Whether this connection asked the backend to revoke the device.
    revoke_asked: bool,
    /// The pages' peers the backend introduced on this connection
    /// (`crates/runner-direct/src/lib.rs`), which end with it.
    direct: Direct,
    /// The user's port forwards on this connection, which end with it.
    forwarding: Option<crate::forward::Forwarding>,
    /// The backend's requests to exposed ports on this connection, which
    /// end with it.
    exposure: Option<crate::exposure::Exposure>,
    /// The manifests installed on this connection, which end with it.
    manifests: Manifests,
    /// The live input of each command that takes some, which a task of the
    /// connection feeds to it.
    feeds: HashMap<String, Feed>,
    /// The message being handled keeps its place in its stream until it
    /// is let go of: a command's input does so until the command reads it.
    held: Option<Held>,
    /// What the backend asked of commands whose start has not arrived yet.
    pending: Pending,
}

/// Serves one connection until it ends, then ends everything it owns; the
/// jobs `kept` holds run on.
pub async fn serve(
    registered: &Registered,
    kept: &mut Kept,
    mut transport: Transport,
) -> io::Result<End> {
    let live = Live {
        control: transport.outgoing.clone(),
        closed: transport.cancellation(),
    };
    let host = HostServer::new(
        transport.outgoing.clone(),
        registered.cwd.clone(),
        registered.pipes.clone(),
    );
    let streams = ServiceStreams::new(
        kept.handle.clone(),
        transport.outgoing.clone(),
        registered.pipes.clone(),
        registered.services.clone(),
        registered.management.draining.clone(),
        transport.cancellation().child_token(),
    );
    let (listed, forwards) = crate::forward::forward_list();
    let operations = HostOperations {
        forwards,
        home: registered.runner.identity.home_dir.clone(),
        watches: host.watches().clone(),
        streams: streams.opener(),
        control: tokio::runtime::Handle::current(),
        backend: transport.outgoing.clone(),
        closed: transport.cancellation(),
    };
    let (direct, driver) = demi_runner_direct::direct(Arc::new(operations), Addresses::Interfaces);
    // The peers' thread ends once the connection lets go of `direct`.
    driver.spawn()?;
    let forwarding = crate::forward::Forwarding::new(
        listed,
        direct.clone(),
        registered.pipes.clone(),
        transport.outgoing.clone(),
        transport.cancellation(),
    );
    let exposure = crate::exposure::Exposure::new(
        registered.pipes.clone(),
        transport.outgoing.clone(),
        transport.cancellation(),
    )?;
    let mut owner = Owner {
        registered,
        kept,
        live,
        work: JoinSet::new(),
        host,
        streams,
        volumes: Volumes::new(
            registered.volumes.clone(),
            transport.outgoing.clone(),
            transport.cancellation(),
        ),
        cached: registered.cached.clone(),
        revoke_asked: false,
        direct,
        forwarding: Some(forwarding),
        exposure: Some(exposure),
        manifests: Manifests::default(),
        feeds: HashMap::new(),
        held: None,
        pending: Pending::default(),
    };
    let result = owner.run(&mut transport).await;
    owner.close().await;
    let closed = transport.close().await;
    // A connection that failed fails the owner too, whose own error, such
    // as a queue that closed with it, says less than why it failed.
    match (result, closed) {
        (_, Err(error)) | (Err(error), _) => Err(error),
        (Ok(end), Ok(())) => Ok(end),
    }
}

impl Owner<'_> {
    async fn run(&mut self, transport: &mut Transport) -> io::Result<End> {
        let hello = wire::encode(&wire::Outbound::Hello {
            protocol: wire::VERSION,
            device_token: self.registered.token.borrow().clone(),
            runner: self.registered.runner.clone(),
            instance: self.registered.instance,
            jobs: self.kept.hello_jobs(),
        })
        .map_err(io::Error::other)?;
        transport.hello(hello).await?;
        let management = self.registered.management.clone();
        // The idle rule reads how many jobs the runner runs: once the
        // connection is online (`online`) and now and then after.
        let mut counts = tokio::time::interval_at(
            tokio::time::Instant::now() + wire::JOB_COUNT_INTERVAL,
            wire::JOB_COUNT_INTERVAL,
        );
        counts.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = management.stop.cancelled() => return Ok(End::Stopped),
                // Asked to remove itself, the runner asks its backend to
                // revoke the device, which ends the connection with
                // `revoked` (`crates/runner/src/removal.rs`).
                _ = management.removing.cancelled(), if !self.revoke_asked && management.phase() == Phase::Online => {
                    self.revoke_asked = true;
                    let revoke = wire::encode(&wire::Outbound::Revoke {}).map_err(io::Error::other)?;
                    self.send(revoke).await?;
                }
                // Draining waits for the jobs that run (`crates/runner/src/connection/mod.rs`).
                _ = management.draining.cancelled(), if self.kept.jobs.is_empty() => {
                    return Ok(End::Stopped);
                }
                // Before the backend's messages: a call registers before it
                // sends the call, so a reply queued beside its registration
                // finds the call.
                event = self.kept.next() => {
                    let control = self.live.control.clone();
                    self.kept.handle(event, Some(&control), self.registered).await;
                }
                Some(work) = self.work.join_next() => {
                    self.worked(work.expect("connection work does not panic")).await?;
                }
                _ = counts.tick(), if management.phase() == Phase::Online => {
                    self.report_job_count().await?;
                }
                Ok(()) = self.cached.changed(), if management.phase() == Phase::Online => {
                    self.report_installed().await?;
                }
                message = transport.input.recv() => match message {
                    Some((message, held)) => {
                        self.held = Some(held);
                        let routed = self.route(message).await;
                        // Let go of the message, unless a command's input
                        // took it.
                        self.held = None;
                        if let Some(end) = routed? {
                            return Ok(end);
                        }
                    }
                    None => return Ok(End::Disconnected),
                },
            }
        }
    }

    /// Reports what the artifact cache holds.
    async fn report_installed(&mut self) -> io::Result<()> {
        let artifacts = self.cached.borrow_and_update().clone();
        let frame =
            wire::encode(&wire::Outbound::Installed { artifacts }).map_err(io::Error::other)?;
        self.send(frame).await
    }

    /// Queues a frame for the backend, waiting for room.
    async fn send(&self, frame: wire::Frame) -> io::Result<()> {
        self.live
            .control
            .send(frame)
            .await
            .map_err(|_| io::Error::other("host connection closed"))
    }

    /// The backend took this connection: the jobs' messages and their
    /// commands' requests go to it, after what a connection that ended may
    /// have lost of the jobs not released: their media's announcements and
    /// the exits of those that ended.
    async fn online(&mut self) -> io::Result<()> {
        let announcements = self.kept.connected(self.live.clone());
        // The jobs that outlived the connection before this one take their
        // input from it.
        for job in self.kept.jobs.running_jobs() {
            self.take_input(WorkId::Job(job));
        }
        for announcement in announcements {
            self.send(announcement).await?;
        }
        self.report_job_count().await?;
        Ok(())
    }

    /// Tells the backend how many jobs the runner runs now.
    async fn report_job_count(&mut self) -> io::Result<()> {
        let count = wire::encode(&wire::Outbound::JobCount {
            jobs: self.kept.jobs.job_count() as u64,
        })
        .map_err(io::Error::other)?;
        self.send(count).await
    }

    async fn worked(&mut self, work: Work) -> io::Result<()> {
        match work {
            Work::Installed {
                installation,
                hash,
                result,
            } => {
                // A manifest that is not the one its message named is none.
                let result = result.and_then(|installed| {
                    if installed.manifest.hash == hash {
                        Ok(installed)
                    } else {
                        Err(io::Error::other("the manifest is not the one its hash names"))
                    }
                });
                let error = result.as_ref().err().map(ToString::to_string);
                let installed = self.manifests.installed(installation, result);
                // The backend starts the jobs that name it after this. A
                // connection that ended needs no answer.
                if let Ok(answer) = wire::encode(&wire::Outbound::ManifestInstalled { hash, error }) {
                    let _ = self.send(answer).await;
                }
                installed?;
            }
            Work::Stored(result) => result?,
            Work::Fed(id) => {
                // Unless a later task feeds the command now.
                if self.feeds.get(&id).is_some_and(|feed| feed.frames.is_closed()) {
                    self.feeds.remove(&id);
                }
            }
            Work::Paired { result, name } => {
                result?;
                // Said once the token is stored, so a runner that reports
                // itself paired keeps its pairing.
                tracing::warn!("online");
                self.registered.announce_paired(&name);
            }
            Work::Done => {}
        }
        Ok(())
    }

    /// Routes one message; `Some` ends the connection.
    async fn route(&mut self, message: Inbound) -> io::Result<Option<End>> {
        if self.kept.relay.route(&message) {
            return Ok(None);
        }
        let registered = self.registered;
        let management = &registered.management;
        match message {
            Inbound::HelloOk {
                device_id,
                device_name,
                conversations,
                archived,
            } => {
                // Before any job of this connection starts, so a
                // conversation made since the backend listed them keeps the
                // directory its first job makes.
                if let Err(error) = registered.scratch.keep_only(&conversations, &archived).await {
                    // The next connection tries again.
                    tracing::warn!("the scratch directories could not be listed: {error}");
                }
                let state = registered.state.clone();
                let config = RunnerConfig {
                    backend_url: registered.backend.url().clone(),
                    device_id: Some(device_id.try_into().map_err(io::Error::other)?),
                };
                self.work
                    .spawn(async move { Work::Stored(state.write_config(&config).await) });
                management.set_phase(Phase::Online);
                tracing::warn!("online");
                registered.announce_paired(&device_name);
                self.online().await?;
                self.report_installed().await?;
            }
            Inbound::ClaimPending { claim_token } if management.phase() != Phase::Online => {
                management.set_phase(Phase::ClaimPending);
                // The code is a secret for the person at the console; the log
                // only says that one is waiting.
                management.tell_code(&claim_token);
                tracing::info!("waiting to be paired");
            }
            Inbound::Claimed {
                device_token,
                device_name,
            } if management.phase() != Phase::Online => {
                registered.token.send_replace(Some(device_token.clone()));
                let state = registered.state.clone();
                self.work.spawn(async move {
                    Work::Paired {
                        result: state.write_token(&device_token).await,
                        name: device_name,
                    }
                });
                management.set_phase(Phase::Online);
                self.online().await?;
                // A Host paired again may hold artifacts from before.
                self.report_installed().await?;
            }
            Inbound::HelloError { code, reason } => {
                tracing::warn!("registration refused ({code}): {reason}");
                if code == wire::HelloErrorCode::AlreadyConnected {
                    return Ok(Some(End::Disconnected));
                }
                // A device revoked while its runner was away, or a backend
                // that lost its data: the runner cannot tell them apart, so
                // it stops and leaves its removal to the person.
                if code == wire::HelloErrorCode::UnknownDevice
                    && let Some(removal) = &registered.removal
                {
                    crate::console::line(format_args!(
                        "demi-runner: this device is no longer paired with {}; to remove this runner, run: {removal}",
                        registered.backend.url()
                    ));
                }
                management.set_phase(Phase::Rejected);
                return Ok(Some(End::Rejected(code)));
            }
            Inbound::Revoked { projects } if management.phase() == Phase::Online => {
                management.settle(Revocation::Revoked {
                    projects: projects.clone(),
                });
                return Ok(Some(End::Revoked(projects)));
            }
            message if management.phase() == Phase::Online => self.message(message).await?,
            _ => {
                return Err(io::Error::other(
                    "backend work arrived before authentication",
                ));
            }
        }
        Ok(None)
    }

    /// Downloads runner release `release`, whose executable `expected`
    /// describes, into the installation while the runner serves, and tells
    /// the backend how it went; the Host log keeps a failure (`crates/runner/src/update.rs`). A connection that ends stops the download, whose
    /// bytes the next download resumes from.
    fn prefetch(&mut self, release: String, expected: demi_shared_artifacts::Digest) {
        let installed = self.registered.installed.clone();
        let backend = self.registered.backend.url().clone();
        let token = self.registered.token.borrow().clone();
        let control = self.live.control.clone();
        let closed = self.live.closed.clone();
        self.work.spawn(async move {
            let placed = match &installed {
                Some(installed) => {
                    installed
                        .prefetch(&backend, token.as_ref(), &release, &expected, &closed)
                        .await
                }
                None => Err(io::Error::other(
                    "this runner was not installed by an installer, so it does not update itself",
                )),
            };
            if closed.is_cancelled() {
                return Work::Done;
            }
            let reply = match placed {
                Ok(()) => {
                    tracing::info!("runner release {release} is downloaded ahead");
                    wire::Outbound::Prefetched { release }
                }
                Err(error) => {
                    tracing::warn!("runner release {release} was not downloaded ahead: {error}");
                    wire::Outbound::PrefetchFailed {
                        release,
                        reason: error.to_string(),
                    }
                }
            };
            match wire::encode(&reply) {
                Ok(reply) => {
                    // A backend that went away needs no answer.
                    let _ = control.send(reply).await;
                }
                Err(error) => tracing::warn!("invalid prefetch answer: {error}"),
            }
            Work::Done
        });
    }

    async fn message(&mut self, message: Inbound) -> io::Result<()> {
        match message {
            Inbound::ConversationRelease {
                id,
                conversation_id,
                over,
            } => {
                let services = self.registered.services.clone();
                let scratch = self.registered.scratch.clone();
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    // The services end what they hold; the runner holds no
                    // files of the conversation (`crates/backend-user-shard/src/lifecycle/conversations.rs`).
                    let result = tokio::select! {
                        _ = closed.cancelled() => return Work::Done,
                        result = services.release_conversation(&conversation_id) => result,
                    };
                    // Its recordings end with their devices and tabs, complete,
                    // as the services release; an archived or deleted
                    // conversation then loses them.
                    if over {
                        scratch.forget_recordings(&conversation_id).await;
                    }
                    match wire::encode(&wire::Outbound::ConversationReleased {
                        id,
                        error: result.err(),
                    }) {
                        Ok(reply) => {
                            // A disconnected backend no longer needs an acknowledgement.
                            let _ = control.send(reply).await;
                        }
                        Err(error) => tracing::warn!("invalid release acknowledgement: {error}"),
                    }
                    Work::Done
                });
            }
            Inbound::PackageRelease { id, packages } => {
                let services = self.registered.services.clone();
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    tokio::select! {
                        _ = closed.cancelled() => return Work::Done,
                        () = services.stop_packages(packages) => {}
                    }
                    match wire::encode(&wire::Outbound::PackageReleased { id }) {
                        Ok(reply) => {
                            // A disconnected backend no longer needs an acknowledgement.
                            let _ = control.send(reply).await;
                        }
                        Err(error) => tracing::warn!("invalid release acknowledgement: {error}"),
                    }
                    Work::Done
                });
            }
            Inbound::Prefetch {
                release,
                sha256,
                size,
            } => self.prefetch(release, demi_shared_artifacts::Digest { size, sha256 }),
            Inbound::Manifest { hash, manifest } => {
                let installing = self.manifests.install(
                    manifest,
                    self.registered.paths.clone(),
                    self.registered.services.clone(),
                    self.registered.reserved.clone(),
                );
                self.work.spawn(async move {
                    let (installation, result) = installing.await;
                    Work::Installed {
                        installation,
                        hash,
                        result,
                    }
                });
            }
            Inbound::JobRead { id, job_id, output } => {
                let output_of = self.kept.directories.output(&job_id);
                let pipes = self.registered.pipes.clone();
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    read_job(output_of, id, output, pipes, control, closed).await;
                    Work::Done
                });
            }
            Inbound::JobMediaRead {
                id,
                job_id,
                numbers,
                output,
            } => {
                let paths = numbers
                    .into_iter()
                    .map(|number| self.kept.directories.medium(&job_id, number))
                    .collect();
                let pipes = self.registered.pipes.clone();
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    read_media(paths, id, output, pipes, control, closed).await;
                    Work::Done
                });
            }
            Inbound::JobRelease { job_id } => {
                let directories = self.kept.directories.clone();
                self.work.spawn(async move {
                    directories.release(&job_id).await;
                    Work::Done
                });
            }
            Inbound::Sync { id } => self.volumes.sync(id)?,
            message if message.fs_request_id().is_some() => self.host.handle_filesystem(message)?,
            message if message.git_request_id().is_some() => self.host.handle_git(message)?,
            Inbound::FsSubscribe { .. } | Inbound::FsUnsubscribe { .. } | Inbound::FsRenew { .. } => {
                self.host.handle_watch(message)?
            }
            Inbound::ServiceOpen { .. } => self.streams.handle_open(message)?,
            Inbound::DirectOffer {
                id,
                peer,
                sdp,
                introduction,
                stun,
            } => {
                let direct = self.direct.clone();
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    // The answer goes first; the candidates the peer finds
                    // after it follow until the peer ends.
                    let (reply, mut candidates) =
                        match direct.offer(peer.clone(), sdp, introduction, stun).await {
                            Ok(answered) => (
                                wire::Outbound::DirectAnswer { id, sdp: answered.sdp },
                                Some(answered.candidates),
                            ),
                            Err(refused) => (
                                wire::Outbound::DirectRefused {
                                    id,
                                    code: refused.code,
                                    message: refused.message,
                                },
                                None,
                            ),
                        };
                    if !send_control(&control, &closed, &reply).await {
                        return Work::Done;
                    }
                    while let Some(candidate) = match &mut candidates {
                        Some(candidates) => candidates.recv().await,
                        None => None,
                    } {
                        let found = wire::Outbound::DirectCandidate {
                            peer: peer.clone(),
                            candidate,
                        };
                        if !send_control(&control, &closed, &found).await {
                            break;
                        }
                    }
                    Work::Done
                });
            }
            Inbound::DirectCandidate { peer, candidate } => self.direct.candidate(&peer, candidate),
            Inbound::Forwards { .. }
            | Inbound::ForwardRetry { .. }
            | Inbound::ForwardConnect { .. }
            | Inbound::ForwardOpened { .. }
            | Inbound::ForwardRefused { .. }
            | Inbound::DirectDialed { .. }
            | Inbound::DirectDialRefused { .. } => {
                if let Some(forwarding) = &self.forwarding {
                    // Each of these is a forward's message.
                    let _ = forwarding.handle(message);
                }
            }
            Inbound::HttpRequest { .. } => {
                if let Some(exposure) = &mut self.exposure {
                    // It is an exposed port's request.
                    let _ = exposure.handle(message);
                }
            }
            Inbound::DirectClose { peer } => self.direct.close(&peer),
            // The relay probe is answered at once, ahead of the connection's
            // bulk output, so its round trip is the path's own.
            Inbound::DirectPing { peer, id } => {
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    send_control(&control, &closed, &wire::Outbound::DirectPong { peer, id }).await;
                    Work::Done
                });
            }
            Inbound::LogRead {
                id,
                since,
                limit,
                source,
            } => {
                let log = self.registered.log.clone();
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    let query = host_log::Query {
                        since,
                        limit: limit as usize,
                        source,
                    };
                    let reply = match log.read(query).await {
                        Ok(page) => wire::encode(&wire::Outbound::LogLines {
                            id: id.clone(),
                            lines: page.lines.into_iter().map(Into::into).collect(),
                            next: page.next,
                        }),
                        Err(error) => wire::encode(&wire::Outbound::LogError {
                            id: id.clone(),
                            message: error.to_string(),
                        }),
                    };
                    match reply {
                        Ok(reply) => {
                            tokio::select! {
                                _ = closed.cancelled() => {},
                                // A disconnected backend no longer waits for the lines.
                                _ = control.send(reply) => {},
                            }
                        }
                        Err(error) => tracing::warn!("log reply encoding failed: {error}"),
                    }
                    Work::Done
                });
            }
            Inbound::JobStdin { job_id: id, bytes } | Inbound::SpawnStdin { spawn_id: id, bytes } => {
                if bytes.0.len() > wire::STDIN_CHUNK_BYTES {
                    return Err(io::Error::other(format!(
                        "live stdin chunk of {} bytes exceeds {} bytes",
                        bytes.0.len(),
                        wire::STDIN_CHUNK_BYTES
                    )));
                }
                self.feed(&id, Input::Bytes(bytes.0))
            }
            Inbound::JobStdinEnd { job_id: id } | Inbound::SpawnStdinEnd { spawn_id: id } => {
                self.feed(&id, Input::End)
            }
            message => self.task(message).await?,
        }
        Ok(())
    }

    /// Hands a command the next frame of its live input, which its feeding
    /// task gives it as it reads; none once the command ended. The frame
    /// keeps the message's place in its stream until the command has it, so
    /// a command that reads nothing stops the link reading its stream, and
    /// nothing else waits.
    fn feed(&mut self, id: &str, input: Input) {
        let held = self.held.take().unwrap_or_else(Held::nothing);
        let Some(feed) = self.feeds.get(id) else {
            return;
        };
        if feed.frames.send(InputFrame { input, _held: held }).is_err() {
            self.feeds.remove(id);
        }
    }

    /// Starts a job or raw process, or routes its input and signals. A start
    /// that cannot begin is answered with its exit.
    async fn task(&mut self, message: Inbound) -> io::Result<()> {
        if let Inbound::JobStart { job_id, .. } = &message
            && self.kept.jobs.started(job_id)
        {
            return self.started_again(job_id).await;
        }
        let routed = self.start_or_route(&message);
        if routed.is_ok() {
            match &message {
                Inbound::JobStart { job_id, .. } => {
                    let work = WorkId::Job(job_id.clone());
                    self.take_input(work.clone());
                    self.apply_pending(&work);
                }
                Inbound::Spawn { spawn_id, .. } => {
                    let work = WorkId::Spawn(spawn_id.clone());
                    self.take_input(work.clone());
                    self.apply_pending(&work);
                }
                _ => {}
            }
        }
        self.registered.management.set_jobs(self.kept.jobs.job_count());
        let Err(error) = routed else {
            return Ok(());
        };
        let work = match message {
            Inbound::JobStart { job_id, .. } => WorkId::Job(job_id),
            Inbound::Spawn { spawn_id, .. } => WorkId::Spawn(spawn_id),
            _ => return Err(error),
        };
        tracing::warn!("backend work could not start: {error}");
        let reply = failure_exit(&work, error.to_string()).map_err(io::Error::other)?;
        self.send(reply).await
    }

    /// Answers a second start of `job` with the job the runner has, since
    /// it starts a job id once in its life (`crates/backend-remote-host/src/device_jobs.rs`): a job that runs goes on; one that ended
    /// answers its exit again while its directory lasts; and one released,
    /// whose end the backend has, or one that failed before it made its
    /// directory, answers that it was started already.
    async fn started_again(&mut self, job: &str) -> io::Result<()> {
        tracing::warn!(job, "a job was started again; it runs once");
        if self.kept.jobs.runs(job) {
            return Ok(());
        }
        let reply = match self.kept.directories.exit(job) {
            Some(exit) => exit,
            None => failure_exit(&WorkId::Job(job.to_owned()), format!("job {job} was started already"))
                .map_err(io::Error::other)?,
        };
        self.send(reply).await
    }

    /// Signals the command `work`, or keeps the signal for its start when
    /// the start has not arrived: the stop is a request on a stream of its
    /// own.
    fn signal(&mut self, work: WorkId, signal: wire::Signal) -> io::Result<()> {
        if self.kept.jobs.has(&work) {
            return self.kept.jobs.signal(&work, signal);
        }
        let started = match &work {
            WorkId::Job(job) => self.kept.jobs.started(job),
            WorkId::Spawn(_) => false,
        };
        if !started {
            self.pending.signal(&work, signal);
        }
        Ok(())
    }

    /// Gives a command that started what was asked of it before.
    fn apply_pending(&mut self, work: &WorkId) {
        let Some(asked) = self.pending.take(work) else {
            return;
        };
        if let Some(follow) = asked.follow {
            self.kept.jobs.follow(work, follow);
        }
        for signal in asked.signals {
            if let Err(error) = self.kept.jobs.signal(work, signal) {
                tracing::warn!("a stop that came before its start was not delivered: {error}");
            }
        }
    }

    fn start_or_route(&mut self, message: &Inbound) -> io::Result<()> {
        let registered = self.registered;
        let draining = registered.management.draining.is_cancelled();
        let demi_home = || {
            (
                "DEMI_HOME".to_owned(),
                registered.state.root.to_string_lossy().into_owned(),
            )
        };
        match message {
            Inbound::JobStart { .. } | Inbound::Spawn { .. } if draining => {
                Err(io::Error::other("runner is draining for upgrade"))
            }
            Inbound::Spawn {
                spawn_id,
                command,
                args,
                cwd,
                env,
                kill_process_group,
                inherit_env,
                descriptors,
            } => {
                let mut values = if env.is_none() || inherit_env == &Some(true) {
                    registered.env.clone()
                } else {
                    BTreeMap::new()
                };
                for (name, value) in env.iter().flatten() {
                    match value {
                        Some(value) => {
                            values.insert(name.clone(), value.clone());
                        }
                        None => {
                            values.remove(name);
                        }
                    }
                }
                values.extend([demi_home()]);
                self.kept.jobs.start(TaskSpec {
                    id: spawn_id.clone(),
                    cwd: cwd
                        .as_ref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| registered.cwd.clone()),
                    env: values,
                    command: TaskCommand::Process {
                        command: command.clone(),
                        args: args.clone().unwrap_or_default(),
                        process_group: kill_process_group.unwrap_or(false),
                        descriptors: descriptors
                            .iter()
                            .flatten()
                            .map(|descriptor| {
                                (descriptor.fd, bytes::Bytes::from(descriptor.bytes.0.clone()))
                            })
                            .collect(),
                    },
                })
            }
            Inbound::JobStart {
                job_id,
                manifest_hash,
                context,
                viewable,
                script,
                cwd,
                env,
                stdin,
                stdout,
            } => {
                let mut values = registered.env.clone();
                values.extend(env.clone());
                // The login profile the job reads and its `$HOME` name one
                // home (`crates/runner-shell/src/lib.rs`), also when neither the
                // request nor the device's environment names one.
                values
                    .entry("HOME".to_owned())
                    .or_insert_with(|| registered.runner.identity.home_dir.clone());
                values.extend([demi_home()]);
                self.kept.jobs.start(TaskSpec {
                    id: job_id.clone(),
                    cwd: PathBuf::from(cwd),
                    env: values,
                    command: TaskCommand::Shell {
                        script: script.clone(),
                        conversation: context.conversation.clone(),
                        caller: context.caller.clone(),
                        stdin: stdin.clone(),
                        stdout: stdout.clone(),
                        // The job pins the manifest its start names as the
                        // connection has it now; later ones leave it be.
                        commands: manifest_hash.clone().map(|hash| {
                            (self.manifests.pin(hash), context.clone(), viewable.clone())
                        }),
                    },
                })
            }
            Inbound::SpawnKill { spawn_id, signal } => self.signal(
                WorkId::Spawn(spawn_id.clone()),
                signal.unwrap_or(wire::Signal::Terminate),
            ),
            Inbound::JobFollow { job_id, follow } => {
                let work = WorkId::Job(job_id.clone());
                if self.kept.jobs.has(&work) {
                    self.kept.jobs.follow(&work, *follow);
                } else if !self.kept.jobs.started(job_id) {
                    self.pending.follow(&work, *follow);
                }
                Ok(())
            }
            Inbound::JobKill { job_id, signal } => {
                self.kept.contexts.cancel(job_id);
                self.signal(
                    WorkId::Job(job_id.clone()),
                    signal.unwrap_or(wire::Signal::Terminate),
                )
            }
            _ => Err(io::Error::other("unexpected backend message")),
        }
    }

    /// Feeds the live input the backend sends for the task `work`, from now
    /// on, to the task: what waits for a task that reads none is held by the
    /// link, which stops reading that command's stream, and nothing else.
    fn take_input(&mut self, work: WorkId) {
        let Some(sender) = self.kept.jobs.input_sender(&work) else {
            return;
        };
        let (WorkId::Job(id) | WorkId::Spawn(id)) = &work;
        let (frames, queue) = mpsc::unbounded_channel();
        self.feeds.insert(id.clone(), Feed { frames });
        let id = id.clone();
        let feeding = Feeding {
            queue,
            input: sender,
            closed: self.live.closed.clone(),
        };
        self.work.spawn(async move {
            feeding.run().await;
            Work::Fed(id)
        });
    }

    /// Ends everything the connection owns: requests, raw processes and
    /// streams stop; the jobs and the services run on (`crates/runner/src/connection/kept.rs`).
    async fn close(&mut self) {
        self.live.closed.cancel();
        self.kept.disconnected();
        self.kept.jobs.end_spawns();
        // Without the backend the runner can no longer hear that a page
        // went away (`crates/runner-direct/src/lib.rs`), and its
        // forwards end with the connection that told it of them.
        self.forwarding = None;
        self.exposure = None;
        self.direct.close_all();
        tokio::join!(self.host.close(), self.streams.close(), self.volumes.close());
        // State writes finish, since a claimed token must not be lost, and a
        // manifest install is kept, since jobs wait for it; the other work
        // sees the connection closed and ends at once.
        while let Some(work) = self.work.join_next().await {
            if let Ok(work @ Work::Installed { .. }) = work
                && let Err(error) = self.worked(work).await
            {
                tracing::warn!("the manifest was not installed: {error}");
            }
        }
    }
}

/// A frame of a command's live input, which holds its message's place in its
/// stream until the command has it.
struct InputFrame {
    input: Input,
    _held: Held,
}

enum Input {
    Bytes(Vec<u8>),
    End,
}

/// Where the owner puts the live input of one command.
struct Feed {
    frames: mpsc::UnboundedSender<InputFrame>,
}

/// The task that gives a command the live input the owner put for it, in
/// order, as the command reads it. The frames waiting are those whose
/// messages the link still holds, at most a few of one stream, so the queue
/// is bounded by what the link lets the stream have ahead. It ends with the
/// command, the input's end or the connection's.
struct Feeding {
    queue: mpsc::UnboundedReceiver<InputFrame>,
    input: InputSender,
    closed: CancellationToken,
}

impl Feeding {
    async fn run(mut self) {
        loop {
            let frame = tokio::select! {
                () = self.closed.cancelled() => return,
                () = self.input.closed() => return,
                frame = self.queue.recv() => frame,
            };
            let Some(InputFrame { input, _held }) = frame else {
                return;
            };
            match input {
                Input::End => {
                    tokio::select! {
                        () = self.closed.cancelled() => {}
                        _ = self.input.end() => {}
                    }
                    return;
                }
                Input::Bytes(bytes) => {
                    let fed = tokio::select! {
                        () = self.closed.cancelled() => return,
                        () = self.input.closed() => return,
                        fed = self.input.bytes(bytes.into()) => fed,
                    };
                    if !fed {
                        return;
                    }
                }
            }
        }
    }
}

/// What the backend asked of commands whose start has not arrived yet: a
/// stop, a follow and a release are requests on streams of their own, which
/// can reach the runner before the start on the command's stream. It is
/// kept for the start, within a few seconds and a few hundred commands.
#[derive(Default)]
struct Pending {
    commands: HashMap<WorkId, (Instant, Asked)>,
}

#[derive(Default)]
struct Asked {
    signals: Vec<wire::Signal>,
    follow: Option<bool>,
}

/// How long what was asked of a command waits for its start.
const PENDING_FOR: Duration = Duration::from_secs(30);
/// The most commands and signals of one command that wait for a start.
const PENDING_COMMANDS: usize = 256;
const PENDING_SIGNALS: usize = 4;

impl Pending {
    fn asked(&mut self, work: &WorkId) -> &mut Asked {
        let now = Instant::now();
        self.commands.retain(|_, (at, _)| now.duration_since(*at) < PENDING_FOR);
        if !self.commands.contains_key(work) && self.commands.len() >= PENDING_COMMANDS {
            // Whatever is oldest goes: a runner holds no more than this.
            if let Some(oldest) = self
                .commands
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(work, _)| work.clone())
            {
                self.commands.remove(&oldest);
            }
        }
        &mut self.commands.entry(work.clone()).or_insert((now, Asked::default())).1
    }

    fn signal(&mut self, work: &WorkId, signal: wire::Signal) {
        let asked = self.asked(work);
        if asked.signals.len() < PENDING_SIGNALS {
            asked.signals.push(signal);
        }
    }

    fn follow(&mut self, work: &WorkId, follow: bool) {
        self.asked(work).follow = Some(follow);
    }

    /// What was asked of `work` before its start, taken.
    fn take(&mut self, work: &WorkId) -> Option<Asked> {
        self.commands.remove(work).map(|(_, asked)| asked)
    }
}

/// Answers a `job_media_read` and streams the media the job keeps at
/// `paths` into its pipe, each after its length; a medium the job does not
/// keep, or a job whose directory is gone, answers that it is not read.
async fn read_media(
    paths: Vec<Option<std::path::PathBuf>>,
    id: String,
    pipe: wire::PipeRef,
    pipes: PipeClient,
    control: Outgoing,
    closed: CancellationToken,
) {
    let targets = paths
        .into_iter()
        .map(|path| {
            path.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "the job keeps no media: it is unknown or released",
                )
            })
        })
        .collect();
    let mut opened = files::open_files(targets, None, &closed).await;
    for medium in &mut opened {
        if let Err(error) = medium
            && error.kind() == io::ErrorKind::NotFound
        {
            *error = io::Error::new(io::ErrorKind::NotFound, "the job keeps no such medium");
        }
    }
    let reply = wire::encode(&wire::Outbound::JobMediaRead {
        id,
        media: files::file_reads(&opened),
    });
    match reply {
        Ok(reply) => {
            tokio::select! {
                _ = closed.cancelled() => return,
                _ = control.send(reply) => {},
            }
        }
        Err(error) => tracing::warn!("job media read reply encoding failed: {error}"),
    }
    let result = pipes.put(&pipe.url, files::framed(opened), &closed).await;
    report_pipe(&control, pipe.id, result, &closed).await;
}

/// Answers a `job_read` and streams the job's kept output, as it stands, into
/// its pipe; a job whose directory is gone answers that nothing flows.
async fn read_job(
    output_of: Option<KeptReader>,
    id: String,
    pipe: wire::PipeRef,
    pipes: PipeClient,
    control: Outgoing,
    closed: CancellationToken,
) {
    let snapshot = match output_of {
        Some(reader) => tokio::task::spawn_blocking(move || reader.snapshot())
            .await
            .map_err(io::Error::other)
            .and_then(|snapshot| snapshot)
            .and_then(|snapshot| snapshot.into_stream()),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the job keeps no output: it is unknown or released",
        )),
    };
    let reply = wire::encode(&wire::Outbound::JobRead {
        id,
        error: snapshot.as_ref().err().map(ToString::to_string),
    });
    match reply {
        Ok(reply) => {
            tokio::select! {
                _ = closed.cancelled() => return,
                _ = control.send(reply) => {},
            }
        }
        Err(error) => tracing::warn!("job read reply encoding failed: {error}"),
    }
    let result = match snapshot {
        Ok(stream) => pipes.put(&pipe.url, stream, &closed).await,
        Err(error) => Err(error),
    };
    report_pipe(&control, pipe.id, result, &closed).await;
}

/// Sends `message` to the backend; false once the connection closed, which
/// takes nothing more.
async fn send_control(
    control: &Outgoing,
    closed: &CancellationToken,
    message: &wire::Outbound,
) -> bool {
    let frame = match wire::encode(message) {
        Ok(frame) => frame,
        Err(error) => {
            // The runner's own messages always encode; one that does not is
            // not sent, and the page falls back to the relay.
            tracing::warn!("a direct message's encoding failed: {error}");
            return true;
        }
    };
    tokio::select! {
        _ = closed.cancelled() => false,
        sent = control.send(frame) => sent.is_ok(),
    }
}
