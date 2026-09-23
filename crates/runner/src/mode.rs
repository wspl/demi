//! One registration owns backend transport, execution contexts and resident services.

use crate::connection::wire::{self as wire, Inbound};
use crate::{
    commands::artifacts::Artifacts,
    commands::contexts::Contexts,
    commands::dispatch::Dispatcher,
    commands::local::Server,
    commands::rpc::Calls,
    commands::streams::ServiceStreams,
    connection::Connection,
    host::HostServer,
    host_log::HostLogReader,
    management::{Management, Phase},
    pipes::PipeClient,
    services::{ServiceHandle, ServiceRegistry},
    state::{ActiveRunner, RunnerConfig, RunnerState},
};
use std::{collections::BTreeMap, io, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub struct Options {
    pub backend: String,
    pub directory: PathBuf,
    /// The Host's log, which `main` opened (`runner.md` § Host log).
    pub log: HostLogReader,
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub runner: wire::RunnerInfo,
    pub token: Option<String>,
    pub volumes: Vec<crate::volumes::ManagedVolume>,
    /// Where shell jobs run (`concurrency.md` § Runner).
    pub shell: crate::shell::ShellRuntime,
}

struct Runtime {
    dispatcher: Arc<Dispatcher>,
    options: Options,
    state: RunnerState,
    token: Arc<RwLock<Option<String>>>,
    contexts: Contexts,
    services: ServiceHandle,
    artifacts: Arc<Artifacts>,
    calls: Arc<Calls>,
    management: Arc<Management>,
    server: Server,
    pipes: PipeClient,
    log: HostLogReader,
}

pub async fn run(options: Options, stop: CancellationToken) -> io::Result<()> {
    let state = RunnerState::open(options.directory.clone()).await?;
    let mut lease = state.lock()?;
    let saved = state.config().await?;
    if let Some(saved) = &saved
        && crate::state::instance_id(&saved.backend_url)?
            != crate::state::instance_id(&options.backend)?
    {
        return Err(io::Error::other(
            "installation is registered to another backend",
        ));
    }
    let token = match &options.token {
        Some(token) => Some(token.clone()),
        None => state.token().await?,
    };
    if options.runner.managed == Some(true) && token.is_none() {
        return Err(io::Error::other("managed runner requires a device token"));
    }
    state
        .write_config(&RunnerConfig {
            backend_url: options.backend.clone(),
            device_id: saved.and_then(|config| config.device_id),
        })
        .await?;
    let token = Arc::new(RwLock::new(token));
    tracing::info!("runner {} started", options.runner.version);
    let registry = ServiceRegistry::new(
        state.root.join("artifacts"),
        options.cwd.clone(),
        options.env.clone(),
    )
    .await
    .map_err(io::Error::other)?;
    let services = registry.handle();
    let contexts = Contexts::new(
        state.root.join("commands"),
        options.executable.clone(),
        services.clone(),
    )
    .await?;
    let artifacts = Artifacts::new(contexts.clone(), crate::services::target().into());
    let pipes = PipeClient::new(&options.backend, token.clone())?;
    let calls = Calls::new(pipes.clone());
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let management = Management::new(secret.clone(), options.runner.version.clone(), stop.clone());
    let dispatcher = Arc::new(Dispatcher {
        contexts: contexts.clone(),
        services: services.clone(),
        resolver: artifacts.clone(),
        calls: calls.clone(),
        management: management.clone(),
    });
    let server = Server::start(dispatcher.clone()).await?;
    lease
        .publish(
            &state,
            &ActiveRunner {
                endpoint: server.endpoint().into(),
                secret,
                release: options.runner.version.clone(),
            },
        )
        .await?;
    let runtime = Runtime {
        dispatcher,
        state,
        token,
        contexts,
        services,
        artifacts,
        calls,
        management,
        server,
        log: options.log.clone(),
        options,
        pipes,
    };
    let outcome = runtime.reconnect().await;
    if runtime.management.draining.is_cancelled() && !runtime.management.stop.is_cancelled() {
        runtime.server.wait_idle().await;
    }
    runtime.contexts.close();
    runtime.calls.detach();
    runtime.artifacts.detach();
    registry.close().await;
    let closed = runtime.server.close().await;
    tracing::info!("runner stopped");
    let released = lease.release();
    outcome.and(closed).and(released)
}

impl Runtime {
    async fn reconnect(&self) -> io::Result<()> {
        let mut delay = Duration::from_millis(250);
        // A backend that stays away fails the same way every few seconds. The
        // console hears each attempt; the log keeps one line per change.
        let mut failure: Option<String> = None;
        loop {
            if self.management.stop.is_cancelled() || self.management.draining.is_cancelled() {
                return Ok(());
            }
            self.management.set_phase(Phase::Connecting);
            if failure.is_none() {
                tracing::info!("connecting to {}", self.options.backend);
            }
            let result = self.connection().await;
            match result {
                Ok(End::Rejected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "backend rejected runner registration",
                    ));
                }
                Ok(End::Stopped) => return Ok(()),
                Ok(End::Disconnected) => {
                    tracing::warn!("backend connection lost");
                    failure = None;
                    delay = Duration::from_millis(250);
                }
                Err(error) => {
                    let text = format!("connection ended: {error}");
                    if failure.as_ref() == Some(&text) {
                        eprintln!("demi-runner: {text}");
                    } else {
                        tracing::warn!("{text}");
                    }
                    failure = Some(text);
                }
            }
            tokio::select! {
                _ = self.management.stop.cancelled() => return Ok(()),
                _ = self.management.draining.cancelled() => return Ok(()),
                _ = tokio::time::sleep(delay) => {},
            }
            delay = (delay * 2).min(Duration::from_secs(10));
        }
    }

    async fn connection(&self) -> io::Result<End> {
        let mut connection = tokio::select! {
            _ = self.management.draining.cancelled() => return Ok(End::Stopped),
            result = Connection::connect(&self.options.backend, self.management.stop.clone()) => result?,
        };
        let host = HostServer::new(
            connection.output.clone(),
            Some(self.dispatcher.clone()),
            self.state.root.join("jobs"),
            self.options.cwd.clone(),
            self.options.env.clone(),
            self.pipes.clone(),
            self.options.shell.clone(),
        );
        self.management.attach(host.tasks.clone());
        let streams = ServiceStreams::new(
            connection.control.clone(),
            self.pipes.clone(),
            self.services.clone(),
            self.artifacts.clone(),
            self.management.draining.clone(),
            connection.cancellation().child_token(),
        );
        let volumes = crate::volumes::Volumes::new(
            self.options.volumes.clone(),
            connection.control.clone(),
            connection.cancellation(),
        );
        self.calls
            .attach(connection.control.clone(), connection.cancellation());
        self.artifacts.attach(connection.control.clone());
        let lifecycle = TaskTracker::new();
        let result = async {
            let hello = wire::encode(&wire::Outbound::Hello {
                protocol: wire::VERSION,
                device_token: self.token.read().await.clone(),
                runner: self.options.runner.clone(),
            })
            .map_err(io::Error::other)?;
            connection
                .control
                .send(hello)
                .await
                .map_err(io::Error::other)?;
            let mut volume_tick = tokio::time::interval(Duration::from_secs(60));
            volume_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let message = tokio::select! {
                    biased;
                    _ = self.management.stop.cancelled() => return Ok(End::Stopped),
                    _ = async {
                        self.management.draining.cancelled().await;
                        host.tasks.wait_idle().await;
                    } => return Ok(End::Stopped),
                    _ = volume_tick.tick(), if self.management.phase() == Phase::Online => {
                        if host.tasks.job_count() == 0 { volumes.poll(); }
                        continue;
                    },
                    message = connection.input.recv() => match message {
                        Some(message) => message,
                        None => return Ok(End::Disconnected),
                    },
                };
                match message {
                    Inbound::HelloOk { device_id } => {
                        self.state
                            .write_config(&RunnerConfig {
                                backend_url: self.options.backend.clone(),
                                device_id: Some(device_id),
                            })
                            .await?;
                        self.management.set_phase(Phase::Online);
                        tracing::warn!("online");
                    }
                    Inbound::ClaimPending { claim_token }
                        if self.management.phase() != Phase::Online =>
                    {
                        self.management.set_phase(Phase::ClaimPending);
                        // The code is a secret for the person at the console;
                        // the log only says that one is waiting.
                        eprintln!("demi-runner: pairing code: {claim_token}");
                        tracing::info!("waiting to be paired");
                    }
                    Inbound::Claimed { device_token }
                        if self.management.phase() != Phase::Online =>
                    {
                        self.state.write_token(&device_token).await?;
                        *self.token.write().await = Some(device_token);
                        self.management.set_phase(Phase::Online);
                        tracing::warn!("online");
                    }
                    Inbound::HelloError { code, reason } => {
                        tracing::warn!("registration refused ({code}): {reason}");
                        if code == wire::HelloErrorCode::AlreadyConnected {
                            return Ok(End::Disconnected);
                        }
                        self.management.set_phase(Phase::Rejected);
                        return Ok(End::Rejected);
                    }
                    Inbound::Ping {} => {
                        connection
                            .control
                            .send(
                                wire::encode(&wire::Outbound::Pong {
                                    jobs: host.tasks.job_count() as u64,
                                })
                                    .map_err(io::Error::other)?,
                            )
                            .await
                            .map_err(io::Error::other)?;
                    }
                    message if self.management.phase() == Phase::Online => {
                        self.message(message, &host, &streams, &connection, &volumes, &lifecycle)
                            .await?
                    }
                    _ => {
                        return Err(io::Error::other(
                            "backend work arrived before authentication",
                        ));
                    }
                }
            }
        }
        .await;
        self.contexts.close();
        self.calls.detach();
        self.artifacts.detach();
        tokio::join!(host.close(), streams.close(), volumes.close());
        self.services.stop_all().await;
        lifecycle.close();
        lifecycle.wait().await;
        self.management.detach();
        let closed = connection.close().await;
        match (result, closed) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(end), Ok(())) => Ok(end),
        }
    }

    async fn message(
        &self,
        message: Inbound,
        host: &HostServer,
        streams: &ServiceStreams,
        connection: &Connection,
        volumes: &crate::volumes::Volumes,
        lifecycle: &TaskTracker,
    ) -> io::Result<()> {
        if self.calls.reply(&message) {
            return Ok(());
        }
        match message {
            Inbound::ConversationRelease {
                id,
                conversation_id,
            } => {
                let services = self.services.clone();
                let output = connection.control.clone();
                let cancel = connection.cancellation();
                lifecycle.spawn(async move {
                    let result = tokio::select! {
                        _ = cancel.cancelled() => return,
                        result = services.release_conversation(&conversation_id) => result,
                    };
                    match wire::encode(&wire::Outbound::ConversationReleased {
                        id,
                        error: result.err(),
                    }) {
                        Ok(reply) => {
                            // A disconnected backend no longer needs an acknowledgement.
                            let _ = output.send(reply).await;
                        }
                        Err(error) => tracing::warn!(
                            "invalid release acknowledgement: {error}"
                        ),
                    }
                });
            }
            Inbound::Manifest { manifest } => {
                self.contexts.install(manifest).await?;
            }
            Inbound::ArtifactLocation {
                id,
                location,
                error,
            } => self.artifacts.reply(&id, location, error),
            Inbound::Sync { id } => volumes.sync(id)?,
            Inbound::VolumeGrown {
                id,
                volume,
                bytes,
                error,
            } => volumes.grown(&id, volume, bytes, error)?,
            message if message.fs_request_id().is_some() => host.handle_filesystem(message)?,
            message if message.git_request_id().is_some() => host.handle_git(message)?,
            Inbound::NetOpen { .. } => host.handle_net(message)?,
            Inbound::ServiceOpen { .. } => streams.handle_open(message)?,
            Inbound::LogRead {
                id,
                since,
                limit,
                source,
            } => {
                let log = self.log.clone();
                let output = connection.control.clone();
                let cancel = connection.cancellation();
                lifecycle.spawn(async move {
                    let query = crate::host_log::Query {
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
                    let reply = match reply {
                        Ok(reply) => reply,
                        Err(error) => {
                            tracing::warn!("log reply encoding failed: {error}");
                            return;
                        }
                    };
                    tokio::select! {
                        _ = cancel.cancelled() => {},
                        // A disconnected backend no longer waits for the lines.
                        _ = output.send(reply) => {},
                    }
                });
            }
            message => {
                let started = matches!(message, Inbound::JobStart { .. } | Inbound::Spawn { .. });
                let setup = async {
                    if started && self.management.draining.is_cancelled() {
                        return Err(io::Error::other("runner is draining for upgrade"));
                    }
                    let mut environment = BTreeMap::from([(
                        "DEMI_HOME".into(),
                        self.state.root.to_string_lossy().into_owned(),
                    )]);
                    let mut lease: Option<Box<dyn Send>> = None;
                    if let Inbound::JobStart {
                        job_id,
                        manifest_hash: Some(hash),
                        env,
                        context,
                        ..
                    } = &message
                    {
                        let (context, lifetime) = self
                            .contexts
                            .create(job_id.clone(), hash, context.clone())
                            .await?;
                        let path = env.get("PATH").or_else(|| self.options.env.get("PATH"));
                        environment.extend(context.environment(
                            self.server.endpoint(),
                            &self.state.root.to_string_lossy(),
                            path.map(String::as_str),
                        )?);
                        lease = Some(Box::new(lifetime));
                    }
                    if let Inbound::JobKill { job_id, .. } = &message {
                        self.contexts.cancel_owner(&format!("job:{job_id}"));
                    }
                    if !host.handle_task(&message, &environment, lease)? {
                        return Err(io::Error::other("unexpected backend message"));
                    }
                    Ok::<_, io::Error>(())
                }
                .await;
                if let Err(error) = setup {
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
                        Inbound::Spawn { spawn_id, .. } => {
                            wire::encode(&wire::Outbound::SpawnExit {
                                spawn_id,
                                exit_code: None,
                                signal: reason,
                                spawn_error: failure,
                            })
                        }
                        _ => return Err(error),
                    }
                    .map_err(io::Error::other)?;
                    connection
                        .control
                        .send(reply)
                        .await
                        .map_err(io::Error::other)?;
                }
            }
        }
        Ok(())
    }
}

enum End {
    Stopped,
    Disconnected,
    Rejected,
}
