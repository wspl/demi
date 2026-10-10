//! The connection's owner: it reads each message and routes it, and it owns
//! what lasts as long as the connection. The jobs outlive it: it serves what
//! the registration keeps while it lasts ([`Kept`]). Every branch of its
//! loop returns without waiting for the work it started, apart from the wait
//! for room in the backend's outbound queue, which the transport drains on
//! its own.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use demi_runner_command_packages::ServiceHandle;
use demi_runner_direct::{Addresses, Direct};
use demi_runner_host::{
    files,
    host::HostServer,
    volumes::{ManagedVolume, Volumes},
};
use demi_runner_jobs::{
    commands::{
        contexts::{self, ContextIndex, ContextPaths, Installation, Installed},
        dispatch::Dispatcher,
        streams::ServiceStreams,
    },
    connection::Live,
    kept_output::KeptReader,
    tasks::{TaskCommand, TaskSpec, WorkId, failure_exit},
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

use super::{Transport, kept::Kept};
use crate::{
    direct::HostOperations,
    host_log::{self, HostLogReader},
    management::{Management, Phase, Revocation},
    state::{RunnerConfig, RunnerState},
};

/// How a connection ended.
pub enum End {
    Stopped,
    Disconnected,
    Rejected,
    /// The backend revoked the device; the projects of these names went
    /// with it.
    Revoked(Vec<String>),
}

/// What each connection takes from its registration.
pub struct Registered {
    pub backend: Backend,
    pub runner: wire::RunnerInfo,
    pub state: Arc<RunnerState>,
    pub token: watch::Sender<Option<DeviceToken>>,
    pub management: Arc<Management>,
    pub services: ServiceHandle,
    /// What the artifact cache holds, which each connection reports
    /// (`native-runtime.md` § Installed artifacts).
    pub cached: watch::Receiver<Vec<wire::HostArtifact>>,
    pub dispatcher: Arc<Dispatcher>,
    /// Where the current connection publishes its live contexts.
    pub index: watch::Sender<Arc<ContextIndex>>,
    pub paths: ContextPaths,
    pub pipes: PipeClient,
    pub log: HostLogReader,
    /// The job root (`runner.md` § Pipes and output).
    pub jobs: PathBuf,
    pub shell: Arc<dyn JobShell>,
    /// The names no declared root command may take: the runner's own and
    /// its shell's builtins.
    pub reserved: BTreeSet<String>,
    /// The local endpoint command clients reach.
    pub endpoint: String,
    /// The default working directory and the environment jobs start from.
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub volumes: Vec<ManagedVolume>,
    /// The command that removes this runner; none for a managed guest's.
    pub removal: Option<String>,
    /// The number this start of the runner drew, which each hello names.
    pub instance: u64,
}

impl Registered {
    /// Tells the person at the console, once per process, that the device
    /// is paired as `name` and how to remove the runner again; an installer
    /// shows them these lines (`runner.md` § Installation, pairing and
    /// removal). A managed guest's runner, which nobody pairs, says nothing.
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
        result: io::Result<Installed>,
    },
    Stored(io::Result<()>),
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
    /// This connection's queue of job and process output.
    output: mpsc::Sender<wire::Frame>,
    work: JoinSet<Work>,
    host: HostServer,
    streams: ServiceStreams,
    volumes: Volumes,
    /// What the artifact cache holds.
    cached: watch::Receiver<Vec<wire::HostArtifact>>,
    /// Whether this connection asked the backend to revoke the device.
    revoke_asked: bool,
    /// The pages' peers the backend introduced on this connection
    /// (`direct-channel.md`), which end with it.
    direct: Direct,
}

/// Serves one connection until it ends, then ends everything it owns; the
/// jobs `kept` holds run on.
pub async fn serve(
    registered: &Registered,
    kept: &mut Kept,
    mut transport: Transport,
) -> io::Result<End> {
    let live = Live {
        control: transport.control.clone(),
        closed: transport.cancellation(),
    };
    let host = HostServer::new(
        transport.output.clone(),
        registered.cwd.clone(),
        registered.pipes.clone(),
    );
    let streams = ServiceStreams::new(
        kept.handle.clone(),
        transport.control.clone(),
        registered.pipes.clone(),
        registered.services.clone(),
        registered.management.draining.clone(),
        transport.cancellation().child_token(),
    );
    let operations = HostOperations {
        home: registered.runner.identity.home_dir.clone(),
        watches: host.watches().clone(),
        streams: streams.opener(),
        control: tokio::runtime::Handle::current(),
        backend: transport.control.clone(),
        closed: transport.cancellation(),
    };
    let (direct, driver) = demi_runner_direct::direct(Arc::new(operations), Addresses::Interfaces);
    // The peers' thread ends once the connection lets go of `direct`.
    driver.spawn()?;
    let mut owner = Owner {
        registered,
        kept,
        live,
        output: transport.output.clone(),
        work: JoinSet::new(),
        host,
        streams,
        volumes: Volumes::new(
            registered.volumes.clone(),
            transport.control.clone(),
            transport.cancellation(),
        ),
        cached: registered.cached.clone(),
        revoke_asked: false,
        direct,
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
        self.send(hello).await?;
        let management = self.registered.management.clone();
        let mut volume_tick = tokio::time::interval(Duration::from_secs(60));
        volume_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = management.stop.cancelled() => return Ok(End::Stopped),
                // Asked to remove itself, the runner asks its backend to
                // revoke the device, which ends the connection with
                // `revoked` (`runner.md` § Installation, pairing and
                // removal).
                _ = management.removing.cancelled(), if !self.revoke_asked && management.phase() == Phase::Online => {
                    self.revoke_asked = true;
                    let revoke = wire::encode(&wire::Outbound::Revoke {}).map_err(io::Error::other)?;
                    self.send(revoke).await?;
                }
                // Draining waits for the jobs that run (`runner.md`
                // § Connection and identity).
                _ = management.draining.cancelled(), if self.kept.jobs.is_empty() => {
                    return Ok(End::Stopped);
                }
                _ = volume_tick.tick(), if management.phase() == Phase::Online => {
                    if self.kept.jobs.job_count() == 0 {
                        self.volumes.poll();
                    }
                }
                // Before the backend's messages: a call registers before it
                // sends the call, so a reply queued beside its registration
                // finds the call.
                event = self.kept.next() => {
                    let control = self.live.control.clone();
                    self.kept.handle(event, Some(&control), self.registered).await;
                }
                Some(work) = self.work.join_next() => {
                    self.worked(work.expect("connection work does not panic"))?;
                }
                Some(()) = self.volumes.checked() => {}
                Ok(()) = self.cached.changed(), if management.phase() == Phase::Online => {
                    self.report_installed().await?;
                }
                message = transport.input.recv() => match message {
                    Some(message) => {
                        if let Some(end) = self.route(message).await? {
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
    /// commands' requests go to it, after the exits of the jobs that ended
    /// and are not released, which a connection that ended may have lost.
    async fn online(&mut self) -> io::Result<()> {
        let exits = self.kept.connected(self.live.clone(), self.output.clone());
        for exit in exits {
            self.output
                .send(exit)
                .await
                .map_err(|_| io::Error::other("host connection closed"))?;
        }
        Ok(())
    }

    fn worked(&mut self, work: Work) -> io::Result<()> {
        match work {
            Work::Installed {
                installation,
                result,
            } => {
                let kept = &mut *self.kept;
                // A later manifest replaced this one before it was kept.
                if installation != kept.installs {
                    return Ok(());
                }
                let installed = result?;
                kept.installation
                    .send_replace(Installation::Ready(installed.manifest.clone()));
                // The new manifest's leases count before the old one's end, so
                // a service both name stays resident.
                kept.installed = Some(installed);
            }
            Work::Stored(result) => result?,
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
            } => {
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
                return Ok(Some(End::Rejected));
            }
            Inbound::Revoked { projects } if management.phase() == Phase::Online => {
                management.settle(Revocation::Revoked {
                    projects: projects.clone(),
                });
                return Ok(Some(End::Revoked(projects)));
            }
            Inbound::Ping {} => {
                let pong = wire::encode(&wire::Outbound::Pong {
                    jobs: self.kept.jobs.job_count() as u64,
                })
                .map_err(io::Error::other)?;
                self.send(pong).await?;
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

    async fn message(&mut self, message: Inbound) -> io::Result<()> {
        match message {
            Inbound::ConversationRelease {
                id,
                conversation_id,
            } => {
                let services = self.registered.services.clone();
                let control = self.live.control.clone();
                let closed = self.live.closed.clone();
                self.work.spawn(async move {
                    // The services end what they hold; the runner holds no
                    // files of the conversation (`resource-lifecycle.md`
                    // § Conversation release).
                    let result = tokio::select! {
                        _ = closed.cancelled() => return Work::Done,
                        result = services.release_conversation(&conversation_id) => result,
                    };
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
            Inbound::Manifest { manifest } => {
                self.kept.installs += 1;
                let installation = self.kept.installs;
                self.kept.installation.send_replace(Installation::Installing);
                let paths = self.registered.paths.clone();
                let services = self.registered.services.clone();
                let reserved = self.registered.reserved.clone();
                self.work.spawn(async move {
                    let result = contexts::install(manifest, &paths, &services, &reserved).await;
                    Work::Installed {
                        installation,
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
            Inbound::VolumeGrown {
                id,
                volume,
                bytes,
                error,
            } => self.volumes.grown(&id, volume, bytes, error)?,
            message if message.fs_request_id().is_some() => self.host.handle_filesystem(message)?,
            message if message.git_request_id().is_some() => self.host.handle_git(message)?,
            Inbound::FsWatch { .. } | Inbound::FsUnwatch { .. } => self.host.handle_watch(message)?,
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
            message => self.task(message).await?,
        }
        Ok(())
    }

    /// Starts a job or raw process, or routes its input and signals. A start
    /// that cannot begin is answered with its exit.
    async fn task(&mut self, message: Inbound) -> io::Result<()> {
        let routed = self.start_or_route(&message);
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
                script,
                cwd,
                env,
                stdin,
                stdout,
            } => {
                let mut values = registered.env.clone();
                values.extend(env.clone());
                // The login profile the job reads and its `$HOME` name one
                // home (`runner.md` § Shell jobs), also when neither the
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
                        stdin: stdin.clone(),
                        stdout: stdout.clone(),
                        commands: manifest_hash.clone().map(|hash| (hash, context.clone())),
                    },
                })
            }
            Inbound::SpawnStdin { spawn_id, bytes } => self
                .kept
                .jobs
                .input(&WorkId::Spawn(spawn_id.clone()), bytes.0.clone().into()),
            Inbound::JobStdin { job_id, bytes } => self
                .kept
                .jobs
                .input(&WorkId::Job(job_id.clone()), bytes.0.clone().into()),
            Inbound::SpawnStdinEnd { spawn_id } => {
                self.kept.jobs.end_input(&WorkId::Spawn(spawn_id.clone()))
            }
            Inbound::JobStdinEnd { job_id } => {
                self.kept.jobs.end_input(&WorkId::Job(job_id.clone()))
            }
            Inbound::SpawnKill { spawn_id, signal } => self.kept.jobs.signal(
                &WorkId::Spawn(spawn_id.clone()),
                signal.unwrap_or(wire::Signal::Terminate),
            ),
            Inbound::JobFollow { job_id, follow } => {
                self.kept.jobs.follow(&WorkId::Job(job_id.clone()), *follow);
                Ok(())
            }
            Inbound::JobKill { job_id, signal } => {
                self.kept.contexts.cancel(job_id);
                self.kept.jobs.signal(
                    &WorkId::Job(job_id.clone()),
                    signal.unwrap_or(wire::Signal::Terminate),
                )
            }
            _ => Err(io::Error::other("unexpected backend message")),
        }
    }

    /// Ends everything the connection owns: requests, raw processes and
    /// streams stop; the jobs and the services run on (`runner.md`
    /// § Command lifetime).
    async fn close(&mut self) {
        self.live.closed.cancel();
        self.kept.disconnected();
        self.kept.jobs.end_spawns();
        // Without the backend the runner can no longer hear that a page
        // went away (`direct-channel.md` § Who may connect).
        self.direct.close_all();
        tokio::join!(self.host.close(), self.streams.close(), self.volumes.close());
        // State writes finish, since a claimed token must not be lost, and a
        // manifest install is kept, since jobs wait for it; the other work
        // sees the connection closed and ends at once.
        while let Some(work) = self.work.join_next().await {
            if let Ok(work @ Work::Installed { .. }) = work
                && let Err(error) = self.worked(work)
            {
                tracing::warn!("the manifest was not installed: {error}");
            }
        }
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
    control: mpsc::Sender<wire::Frame>,
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
    control: mpsc::Sender<wire::Frame>,
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
    control: &mpsc::Sender<wire::Frame>,
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
