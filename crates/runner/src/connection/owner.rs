//! The connection's owner: it reads each message and routes it, and it owns
//! what lasts as long as the connection. Every branch of its loop returns
//! without waiting for the work it started, apart from the wait for room in
//! the backend's outbound queue, which the transport drains on its own.

use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};

use super::{ConnectionHandle, Ended, Relay, Request, Transport, wire};
use crate::{
    commands::{
        contexts::{self, ContextIndex, ContextPaths, ContextTable, Installation, Installed},
        dispatch::Dispatcher,
        streams::ServiceStreams,
    },
    host::HostServer,
    host_log::{self, HostLogReader},
    management::{Management, Phase},
    pipes::PipeClient,
    services::ServiceHandle,
    shell::ShellRuntime,
    state::{RunnerConfig, RunnerState},
    tasks::{Commands, JobConfig, JobTable, TaskCommand, TaskSpec, WorkId},
    volumes::{ManagedVolume, Volumes},
};
use demi_runner_protocol::values::{BackendUrl, DeviceToken};
use wire::Inbound;

/// How a connection ended.
pub enum End {
    Stopped,
    Disconnected,
    Rejected,
}

/// What each connection takes from its registration.
pub struct Registered {
    pub backend: BackendUrl,
    pub runner: wire::RunnerInfo,
    pub state: Arc<RunnerState>,
    pub token: watch::Sender<Option<DeviceToken>>,
    pub management: Arc<Management>,
    pub services: ServiceHandle,
    pub dispatcher: Arc<Dispatcher>,
    /// Where the current connection publishes its live contexts.
    pub index: watch::Sender<Arc<ContextIndex>>,
    pub paths: ContextPaths,
    pub pipes: PipeClient,
    pub log: HostLogReader,
    pub shell: ShellRuntime,
    /// The local endpoint command clients reach.
    pub endpoint: String,
    /// The default working directory and the environment jobs start from.
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub volumes: Vec<ManagedVolume>,
}

/// What the owner's child tasks hand back.
enum Work {
    Installed {
        installation: u64,
        result: io::Result<Installed>,
    },
    Stored(io::Result<()>),
    Done,
}

struct Owner<'r> {
    registered: &'r Registered,
    handle: ConnectionHandle,
    jobs: JobTable,
    contexts: ContextTable,
    /// The manifest jobs see, and the leases of the one installed.
    installation: watch::Sender<Installation>,
    installed: Option<Installed>,
    /// Numbers manifest installs, so only the latest one counts.
    installs: u64,
    relay: Relay,
    watches: JoinSet<Ended>,
    work: JoinSet<Work>,
    host: HostServer,
    streams: ServiceStreams,
    volumes: Volumes,
}

/// Serves one connection until it ends, then ends everything it owns.
pub async fn serve(registered: &Registered, mut transport: Transport) -> io::Result<End> {
    let (handle, mut requests) =
        ConnectionHandle::new(transport.control.clone(), transport.cancellation());
    let (installation, installations) = watch::channel(Installation::Absent);
    let mut owner = Owner {
        registered,
        jobs: JobTable::new(JobConfig {
            output: transport.output.clone(),
            output_dir: registered.state.root.join("jobs"),
            pipes: registered.pipes.clone(),
            shell: registered.shell.clone(),
            commands: Some(Commands {
                dispatcher: registered.dispatcher.clone(),
                connection: handle.clone(),
                installation: installations,
                paths: registered.paths.clone(),
                services: registered.services.clone(),
                endpoint: registered.endpoint.clone(),
                home: registered.state.root.to_string_lossy().into_owned(),
            }),
        }),
        contexts: ContextTable::new(registered.index.clone()),
        installation,
        installed: None,
        installs: 0,
        relay: Relay::default(),
        watches: JoinSet::new(),
        work: JoinSet::new(),
        host: HostServer::new(
            transport.output.clone(),
            registered.cwd.clone(),
            registered.pipes.clone(),
        ),
        streams: ServiceStreams::new(
            handle.clone(),
            registered.pipes.clone(),
            registered.services.clone(),
            registered.management.draining.clone(),
            transport.cancellation().child_token(),
        ),
        volumes: Volumes::new(
            registered.volumes.clone(),
            transport.control.clone(),
            transport.cancellation(),
        ),
        handle,
    };
    let result = owner.run(&mut transport, &mut requests).await;
    // Requests still queued get no answer; their askers see the end.
    drop(requests);
    owner.close().await;
    let closed = transport.close().await;
    match (result, closed) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(end), Ok(())) => Ok(end),
    }
}

impl Owner<'_> {
    async fn run(
        &mut self,
        transport: &mut Transport,
        requests: &mut mpsc::Receiver<Request>,
    ) -> io::Result<End> {
        let hello = wire::encode(&wire::Outbound::Hello {
            protocol: wire::VERSION,
            device_token: self.registered.token.borrow().clone(),
            runner: self.registered.runner.clone(),
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
                // Draining waits for the jobs that run (`runner.md`
                // § Connection and identity).
                _ = management.draining.cancelled(), if self.jobs.is_empty() => {
                    return Ok(End::Stopped);
                }
                _ = volume_tick.tick(), if management.phase() == Phase::Online => {
                    if self.jobs.job_count() == 0 {
                        self.volumes.poll();
                    }
                }
                Some(request) = requests.recv() => self.request(request).await?,
                Some(id) = self.jobs.finished() => self.finished(&id),
                Some(ended) = self.watches.join_next() => {
                    self.relay.ended(ended.expect("relay watches do not panic"));
                }
                Some(work) = self.work.join_next() => {
                    self.worked(work.expect("connection work does not panic"))?;
                }
                Some(()) = self.volumes.checked() => {}
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

    /// Queues a frame for the backend, waiting for room.
    async fn send(&self, frame: wire::Frame) -> io::Result<()> {
        self.handle
            .control
            .send(frame)
            .await
            .map_err(|_| io::Error::other("host connection closed"))
    }

    async fn request(&mut self, request: Request) -> io::Result<()> {
        match request {
            Request::Call { id, events, ended } => {
                self.relay.call(id, events, ended, &mut self.watches);
            }
            Request::Locate {
                owner,
                sha256,
                reply,
                abandoned,
            } => {
                let frame = self
                    .relay
                    .locate(owner, sha256, reply, abandoned, &mut self.watches)
                    .map_err(io::Error::other)?;
                self.send(frame).await?;
            }
            Request::Context {
                context,
                leases,
                reply,
            } => {
                // A job that gave up no longer needs to know.
                let _gone = reply.send(self.contexts.insert(context, leases));
            }
        }
        Ok(())
    }

    fn finished(&mut self, id: &WorkId) {
        if let WorkId::Job(job) = id {
            self.contexts.remove(job);
        }
        self.registered.management.set_jobs(self.jobs.job_count());
    }

    fn worked(&mut self, work: Work) -> io::Result<()> {
        match work {
            Work::Installed {
                installation,
                result,
            } => {
                // A later manifest replaced this one before it was kept.
                if installation != self.installs {
                    return Ok(());
                }
                let installed = result?;
                self.installation
                    .send_replace(Installation::Ready(installed.manifest.clone()));
                // The new manifest's leases count before the old one's end, so
                // a service both name stays resident.
                self.installed = Some(installed);
            }
            Work::Stored(result) => result?,
            Work::Done => {}
        }
        Ok(())
    }

    /// Routes one message; `Some` ends the connection.
    async fn route(&mut self, message: Inbound) -> io::Result<Option<End>> {
        if self.relay.route(&message) {
            return Ok(None);
        }
        let registered = self.registered;
        let management = &registered.management;
        match message {
            Inbound::HelloOk { device_id } => {
                let state = registered.state.clone();
                let config = RunnerConfig {
                    backend_url: registered.backend.clone(),
                    device_id: Some(device_id.try_into().map_err(io::Error::other)?),
                };
                self.work
                    .spawn(async move { Work::Stored(state.write_config(&config).await) });
                management.set_phase(Phase::Online);
                tracing::warn!("online");
            }
            Inbound::ClaimPending { claim_token } if management.phase() != Phase::Online => {
                management.set_phase(Phase::ClaimPending);
                // The code is a secret for the person at the console; the log
                // only says that one is waiting.
                eprintln!("demi-runner: pairing code: {claim_token}");
                tracing::info!("waiting to be paired");
            }
            Inbound::Claimed { device_token } if management.phase() != Phase::Online => {
                registered.token.send_replace(Some(device_token.clone()));
                let state = registered.state.clone();
                self.work
                    .spawn(async move { Work::Stored(state.write_token(&device_token).await) });
                management.set_phase(Phase::Online);
                tracing::warn!("online");
            }
            Inbound::HelloError { code, reason } => {
                tracing::warn!("registration refused ({code}): {reason}");
                if code == wire::HelloErrorCode::AlreadyConnected {
                    return Ok(Some(End::Disconnected));
                }
                management.set_phase(Phase::Rejected);
                return Ok(Some(End::Rejected));
            }
            Inbound::Ping {} => {
                let pong = wire::encode(&wire::Outbound::Pong {
                    jobs: self.jobs.job_count() as u64,
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
                let control = self.handle.control.clone();
                let closed = self.handle.closed().clone();
                self.work.spawn(async move {
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
                self.installs += 1;
                let installation = self.installs;
                self.installation.send_replace(Installation::Installing);
                let paths = self.registered.paths.clone();
                let services = self.registered.services.clone();
                self.work.spawn(async move {
                    let result = contexts::install(manifest, &paths, &services).await;
                    Work::Installed {
                        installation,
                        result,
                    }
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
            Inbound::NetOpen { .. } => self.host.handle_net(message)?,
            Inbound::ServiceOpen { .. } => self.streams.handle_open(message)?,
            Inbound::LogRead {
                id,
                since,
                limit,
                source,
            } => {
                let log = self.registered.log.clone();
                let control = self.handle.control.clone();
                let closed = self.handle.closed().clone();
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
        let started = matches!(message, Inbound::JobStart { .. } | Inbound::Spawn { .. });
        let routed = self.start_or_route(&message);
        self.registered.management.set_jobs(self.jobs.job_count());
        let Err(error) = routed else {
            return Ok(());
        };
        if !started {
            return Err(error);
        }
        tracing::warn!("backend work could not start: {error}");
        let reason = Some(error.to_string());
        let failure = Some(wire::SpawnError {
            kind: wire::SpawnErrorKind::Other,
            detail: None,
        });
        let reply = match message {
            Inbound::JobStart { job_id, .. } => wire::encode(&wire::Outbound::JobExit {
                job_id,
                exit_code: None,
                signal: reason,
                spawn_error: failure,
                cwd: None,
                output: None,
                files: Vec::new(),
                files_truncated: false,
            }),
            Inbound::Spawn { spawn_id, .. } => wire::encode(&wire::Outbound::SpawnExit {
                spawn_id,
                exit_code: None,
                signal: reason,
                spawn_error: failure,
            }),
            _ => unreachable!("only starts are answered"),
        }
        .map_err(io::Error::other)?;
        self.send(reply).await
    }

    fn start_or_route(&mut self, message: &Inbound) -> io::Result<()> {
        let registered = self.registered;
        let draining = registered.management.draining.is_cancelled();
        let home = || {
            (
                "DEMI_HOME".to_owned(),
                registered.state.root.to_string_lossy().into_owned(),
            )
        };
        match message {
            Inbound::JobStart { .. } | Inbound::Spawn { .. } if draining => {
                return Err(io::Error::other("runner is draining for upgrade"));
            }
            Inbound::Spawn {
                spawn_id,
                command,
                args,
                cwd,
                env,
                kill_process_group,
                inherit_env,
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
                values.extend([home()]);
                self.jobs.start(TaskSpec {
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
                values.extend([home()]);
                self.jobs.start(TaskSpec {
                    id: job_id.clone(),
                    cwd: PathBuf::from(cwd),
                    env: values,
                    command: TaskCommand::Shell {
                        script: script.clone(),
                        stdin: stdin.clone(),
                        stdout: stdout.clone(),
                        commands: manifest_hash
                            .clone()
                            .map(|hash| (hash, context.clone())),
                    },
                })
            }
            Inbound::SpawnStdin { spawn_id, bytes } => self
                .jobs
                .input(&WorkId::Spawn(spawn_id.clone()), bytes.0.clone().into()),
            Inbound::JobStdin { job_id, bytes } => self
                .jobs
                .input(&WorkId::Job(job_id.clone()), bytes.0.clone().into()),
            Inbound::SpawnStdinEnd { spawn_id } => {
                self.jobs.end_input(&WorkId::Spawn(spawn_id.clone()))
            }
            Inbound::JobStdinEnd { job_id } => self.jobs.end_input(&WorkId::Job(job_id.clone())),
            Inbound::SpawnKill { spawn_id, signal } => self.jobs.signal(
                &WorkId::Spawn(spawn_id.clone()),
                signal.unwrap_or(wire::Signal::Terminate),
            ),
            Inbound::JobKill { job_id, signal } => {
                self.contexts.cancel(job_id);
                self.jobs.signal(
                    &WorkId::Job(job_id.clone()),
                    signal.unwrap_or(wire::Signal::Terminate),
                )
            }
            _ => Err(io::Error::other("unexpected backend message")),
        }
    }

    /// Ends everything the connection owns: jobs, requests and streams stop,
    /// and losing the backend shuts every service down (`runner.md`
    /// § Command lifetime).
    async fn close(&mut self) {
        self.handle.closed().cancel();
        tokio::join!(
            self.jobs.close(),
            self.host.close(),
            self.streams.close(),
            self.volumes.close()
        );
        // State writes finish, since a claimed token must not be lost; the
        // other work sees the connection closed and ends at once.
        while self.work.join_next().await.is_some() {}
        self.watches.shutdown().await;
        self.installation.send_replace(Installation::Absent);
        self.installed = None;
        self.registered.management.set_jobs(0);
        self.registered.services.stop_all().await;
    }
}
