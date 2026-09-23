//! One registration (`runner.md` § Connection and identity): the
//! installation state and its lock, the device token, the local command
//! endpoint, the service registry and the reconnect loop. Each backend
//! connection in turn is served by its own owner
//! ([`crate::connection::serve`]).

use crate::connection::{self, End, Registered, Transport, wire};
use crate::{
    commands::contexts::{ContextPaths, Contexts},
    commands::dispatch::Dispatcher,
    commands::local::Server,
    host_log::HostLogReader,
    management::{Management, Phase},
    pipes::PipeClient,
    services::ServiceRegistry,
    state::{ActiveRunner, RunnerConfig, RunnerState},
};
use demi_runner_protocol::values::{BackendUrl, DeviceToken};
use std::{collections::BTreeMap, io, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct Options {
    pub backend: BackendUrl,
    pub directory: PathBuf,
    /// The Host's log, which `main` opened (`runner.md` § Host log).
    pub log: HostLogReader,
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub runner: wire::RunnerInfo,
    pub token: Option<DeviceToken>,
    pub volumes: Vec<crate::volumes::ManagedVolume>,
    /// Where shell jobs run (`concurrency.md` § Runner).
    pub shell: crate::shell::ShellRuntime,
}

pub async fn run(options: Options, stop: CancellationToken) -> io::Result<()> {
    let state = RunnerState::open(options.directory.clone()).await?;
    let mut lease = state.lock()?;
    let saved = state.config().await?;
    if let Some(saved) = &saved
        && crate::state::instance_id(&saved.backend_url) != crate::state::instance_id(&options.backend)
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
    // Every pipe request reads the token; only a claim changes it.
    let token = watch::Sender::new(token);
    tracing::info!("runner {} started", options.runner.version);
    let registry = ServiceRegistry::new(
        state.root.join("artifacts"),
        options.cwd.clone(),
        options.env.clone(),
    )
    .await
    .map_err(io::Error::other)?;
    let paths = ContextPaths::new(state.root.join("commands"), options.executable.clone()).await?;
    let index = watch::Sender::new(Arc::default());
    let pipes = PipeClient::new(&options.backend, token.subscribe())?;
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let management = Management::new(secret.clone(), options.runner.version.clone(), stop.clone());
    let dispatcher = Arc::new(Dispatcher {
        contexts: Contexts::new(index.subscribe()),
        services: registry.handle(),
        pipes: pipes.clone(),
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
    let registered = Registered {
        backend: options.backend,
        runner: options.runner,
        state: Arc::new(state),
        token,
        management,
        services: registry.handle(),
        dispatcher,
        index,
        paths,
        pipes,
        log: options.log,
        shell: options.shell,
        endpoint: server.endpoint().into(),
        cwd: options.cwd,
        env: options.env,
        volumes: options.volumes,
    };
    let outcome = reconnect(&registered).await;
    if registered.management.draining.is_cancelled() && !registered.management.stop.is_cancelled()
    {
        server.wait_idle().await;
    }
    registry.close().await;
    let closed = server.close().await;
    tracing::info!("runner stopped");
    let released = lease.release();
    outcome.and(closed).and(released)
}

async fn reconnect(registered: &Registered) -> io::Result<()> {
    let management = &registered.management;
    let mut delay = Duration::from_millis(250);
    // A backend that stays away fails the same way every few seconds. The
    // console hears each attempt; the log keeps one line per change.
    let mut failure: Option<String> = None;
    loop {
        if management.stop.is_cancelled() || management.draining.is_cancelled() {
            return Ok(());
        }
        management.set_phase(Phase::Connecting);
        if failure.is_none() {
            tracing::info!("connecting to {}", registered.backend);
        }
        let result = connection(registered).await;
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
            _ = management.stop.cancelled() => return Ok(()),
            _ = management.draining.cancelled() => return Ok(()),
            _ = tokio::time::sleep(delay) => {},
        }
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Opens one backend connection and serves it until it ends.
async fn connection(registered: &Registered) -> io::Result<End> {
    let management = &registered.management;
    let transport = tokio::select! {
        _ = management.draining.cancelled() => return Ok(End::Stopped),
        result = Transport::connect(&registered.backend, management.stop.clone()) => result?,
    };
    connection::serve(registered, transport).await
}
