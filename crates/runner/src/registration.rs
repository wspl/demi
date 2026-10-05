//! One registration (`runner.md` § Connection and identity): the
//! installation state and its lock, the device token, the local command
//! endpoint, the service registry and the reconnect loop. Each backend
//! connection in turn is served by its own owner
//! ([`crate::connection::serve`]).

use crate::connection::{self, Connected, End, Registered, Transport};
use crate::update::{Installed, Successor};
use crate::{
    host_log::HostLogReader,
    management::{Endpoint, Management, Phase},
    state::{ActiveRunner, RunnerConfig, RunnerState},
};
use demi_runner_command_packages::ServiceRegistry;
use demi_runner_host::volumes::ManagedVolume;
use demi_runner_jobs::commands::{
    contexts::{ContextPaths, Contexts},
    dispatch::Dispatcher,
    local::Server,
};
use demi_runner_process::{job_shell::JobShell, pipes::PipeClient};
use demi_runner_protocol::{
    image::ARTIFACTS_PATH,
    values::{BackendUrl, DeviceToken},
    wire,
};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct Options {
    pub backend: BackendUrl,
    pub directory: PathBuf,
    /// The artifact cache (`native-runtime.md` § Install
    /// artifacts).
    pub artifacts: PathBuf,
    /// The Host's log, which `main` opened (`runner.md` § Host log).
    pub log: HostLogReader,
    /// The job root, where each shell job keeps its output under its
    /// conversation (`runner.md` § Pipes and output).
    pub jobs: PathBuf,
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub runner: wire::RunnerInfo,
    pub token: Option<DeviceToken>,
    pub volumes: Vec<ManagedVolume>,
    /// Runs the jobs' scripts (`concurrency.md` § Runner).
    pub shell: Arc<dyn JobShell>,
    /// The installation an installer made, which updates itself to its
    /// backend's runner release; none for a runner started otherwise.
    pub installed: Option<Installed>,
    /// The command that removes this runner, which it tells its console once
    /// paired; none for a managed guest's.
    pub removal: Option<String>,
}

/// How a registration ended.
pub enum Ending {
    Stopped,
    /// The backend runs another runner release, which is in place: the
    /// successor starts once this registration has let everything go
    /// (`runner.md` § Runner updates).
    Replaced(Successor),
}

pub async fn run(options: Options, stop: CancellationToken) -> io::Result<Ending> {
    let state = RunnerState::open(options.directory.clone()).await?;
    let mut lease = state.lock()?;
    let saved = state.config().await?;
    if let Some(saved) = &saved
        && crate::state::instance_id(&saved.backend_url)
            != crate::state::instance_id(&options.backend)
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
    // A Cloud image preinstalls the command artifacts there. The runner
    // looks on every Host, and a paired device has nothing there; Cloud
    // images are Linux only, so a Windows runner does not look.
    let image = cfg!(unix).then(|| PathBuf::from(ARTIFACTS_PATH));
    let registry = ServiceRegistry::new(
        options.artifacts.clone(),
        image,
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
    });
    let server = Server::start(Arc::new(Endpoint {
        dispatcher: dispatcher.clone(),
        management: management.clone(),
    }))
    .await?;
    // A root command of the runner's name would run the runner, and one of a
    // builtin's name the builtin.
    let mut reserved = options.shell.builtin_names();
    reserved.insert(crate::PROGRAM.into());
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
    let installed = options.installed;
    let registered = Registered {
        backend: options.backend,
        runner: options.runner,
        state: Arc::new(state),
        token,
        management,
        services: registry.handle(),
        cached: registry.installed(),
        dispatcher,
        index,
        paths,
        pipes,
        log: options.log,
        jobs: options.jobs,
        shell: options.shell,
        reserved,
        endpoint: server.endpoint().into(),
        cwd: options.cwd,
        env: options.env,
        volumes: options.volumes,
        removal: options.removal,
        announced: AtomicBool::new(false),
    };
    let outcome = reconnect(&registered, installed.as_ref()).await;
    if registered.management.draining.is_cancelled() && !registered.management.stop.is_cancelled() {
        server.wait_idle().await;
    }
    registry.close().await;
    let closed = server.close().await;
    tracing::info!("runner stopped");
    let released = lease.release();
    let ending = outcome?;
    closed.and(released)?;
    Ok(ending)
}

async fn reconnect(registered: &Registered, installed: Option<&Installed>) -> io::Result<Ending> {
    let management = &registered.management;
    let mut delay = Duration::from_millis(250);
    // A backend that stays away fails the same way every few seconds. The
    // console hears each attempt; the log keeps one line per change.
    let mut failure: Option<String> = None;
    loop {
        if management.stop.is_cancelled() || management.draining.is_cancelled() {
            return Ok(Ending::Stopped);
        }
        management.set_phase(Phase::Connecting);
        if failure.is_none() {
            tracing::info!("connecting to {}", registered.backend);
        }
        let result = connection(registered).await;
        let result = match result {
            Ok(Attempt::Update(update)) => {
                let Some(installed) = installed else {
                    return Err(io::Error::other(format!(
                        "this runner is release {}, and its backend runs release {}: install the backend's runner",
                        registered.runner.version, update.release
                    )));
                };
                tracing::info!("updating to runner release {}", update.release);
                match installed
                    .update(&registered.backend, &update, &management.stop)
                    .await
                {
                    Ok(successor) => return Ok(Ending::Replaced(successor)),
                    Err(error) => Err(io::Error::other(format!(
                        "the update to runner release {} failed: {error}",
                        update.release
                    ))),
                }
            }
            Ok(Attempt::Ended(end)) => Ok(end),
            Err(error) => Err(error),
        };
        match result {
            Ok(End::Rejected) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "backend rejected runner registration",
                ));
            }
            Ok(End::Stopped) => return Ok(Ending::Stopped),
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
            _ = management.stop.cancelled() => return Ok(Ending::Stopped),
            _ = management.draining.cancelled() => return Ok(Ending::Stopped),
            _ = tokio::time::sleep(delay) => {},
        }
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// What one connection attempt came to.
enum Attempt {
    Ended(End),
    /// The backend opens no socket for this runner's release.
    Update(demi_runner_protocol::release::RunnerUpdate),
}

/// Opens one backend connection and serves it until it ends.
async fn connection(registered: &Registered) -> io::Result<Attempt> {
    let management = &registered.management;
    let connected = tokio::select! {
        _ = management.draining.cancelled() => return Ok(Attempt::Ended(End::Stopped)),
        result = Transport::connect(&registered.backend, &registered.runner.version, management.stop.clone()) => result?,
    };
    match connected {
        Connected::Open(transport) => connection::serve(registered, transport)
            .await
            .map(Attempt::Ended),
        Connected::Update(update) => Ok(Attempt::Update(update)),
    }
}
