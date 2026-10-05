//! The runner (`runner.md`): the execution host's program, which registers
//! with the backend and serves its work, and, under a root command's name,
//! the command alias that forwards one command line to it.

mod connection;
mod host_log;
mod management;
mod registration;
mod removal;
mod state;
mod update;

use demi_command_protocol::{LocalInvocation, host_target};
use demi_runner_host::volumes::ManagedVolume;
use demi_runner_process::{
    command_client::{self, RAW, RawCommand, Stdio},
    stdio::{self, standard_file},
};
use demi_runner_protocol::{
    boot::ManagedBoot,
    values::BackendUrl,
    wire::{self, RunnerPlatform},
};
use demi_runner_shell::ShellRuntime;
use registration::{Ending, Options};
use state::RunnerState;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{
    Layer as _, filter::LevelFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

async fn command(root: String, argv: Vec<String>) -> io::Result<u8> {
    let env: BTreeMap<_, _> = std::env::vars().collect();
    let endpoint = env
        .get(command_client::ENDPOINT_ENV)
        .ok_or_else(|| io::Error::other("command requires a runner execution context"))?
        .clone();
    let context = env
        .get(command_client::CONTEXT_ENV)
        .ok_or_else(|| io::Error::other("missing command context"))?
        .clone();
    let stdin = standard_file(0)?;
    let stdout = standard_file(1)?;
    let request = RawCommand::new(
        context,
        root,
        argv,
        stdio::is_live(&stdin, &env)?,
        stdio::stdout_target(&stdout, &env)?,
    )?;
    let invocation = LocalInvocation {
        operation: RAW.into(),
        invocation_id: uuid::Uuid::new_v4().simple().to_string(),
        args: serde_json::to_value(request).map_err(io::Error::other)?,
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        env,
    };
    let stop = CancellationToken::new();
    tokio::select! {
        result = command_client::forward(&endpoint, &invocation, Stdio {
            stdin: tokio::fs::File::from_std(stdin),
            stdout: tokio::fs::File::from_std(stdout),
            stderr: tokio::fs::File::from_std(standard_file(2)?),
        }, stop.clone()) => result.map(|result| result.exit_code),
        result = signal() => {
            result?;
            stop.cancel();
            Ok(130)
        },
    }
}

/// Asks the installation's active runner for its status, to drain, or to
/// remove itself; the status it answers goes to `stdout`.
async fn manage(
    state: RunnerState,
    action: management::Action,
    release: Option<&str>,
    stdout: impl tokio::io::AsyncWrite + Unpin,
) -> io::Result<u8> {
    let active = state.active().await?;
    let request = LocalInvocation {
        operation: management::MANAGE.into(),
        invocation_id: uuid::Uuid::new_v4().simple().to_string(),
        args: serde_json::to_value(management::Request {
            secret: active.secret.clone(),
            action,
        })
        .map_err(io::Error::other)?,
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        env: BTreeMap::new(),
    };
    let completion = command_client::forward(
        &active.endpoint,
        &request,
        Stdio {
            stdin: tokio::io::empty(),
            stdout,
            stderr: tokio::fs::File::from_std(standard_file(2)?),
        },
        CancellationToken::new(),
    )
    .await?;
    if completion.exit_code != 0 {
        return Ok(completion.exit_code);
    }
    match action {
        // The drained runner releases the installation lock as it ends.
        management::Action::Drain | management::Action::Uninstall => loop {
            match state.try_lock()? {
                Some(lease) => {
                    lease.release()?;
                    return Ok(0);
                }
                None => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        },
        management::Action::Status if release.is_some_and(|release| release != active.release) => {
            Ok(3)
        }
        management::Action::Status => Ok(0),
    }
}

/// The installation directory: a managed guest's temporary state, the one
/// `DEMI_HOME` names, or the backend's under the user's home.
fn directory(installation: &Installation, boot: Option<&ManagedBoot>) -> io::Result<PathBuf> {
    if boot.is_some() {
        return Ok(PathBuf::from("/run/demi"));
    }
    if let Some(directory) = &installation.home {
        return Ok(directory.clone());
    }
    let backend = installation
        .backend
        .as_ref()
        .ok_or_else(|| io::Error::other("pass --backend <url> to select an installation"))?;
    Ok(home()?
        .join(".demi/instances")
        .join(state::instance_id(backend)))
}

fn home() -> io::Result<PathBuf> {
    std::env::home_dir()
        .filter(|home| !home.as_os_str().is_empty())
        .ok_or_else(|| io::Error::other("user home is not configured"))
}

/// The runner's own name, under which it runs as the runner rather than as a
/// command alias.
const PROGRAM: &str = "demi-runner";

/// Runs this device as a Demi execution target.
#[derive(clap::Parser)]
#[command(name = PROGRAM, version)]
struct Cli {
    #[command(subcommand)]
    action: Action,
}

#[derive(clap::Subcommand)]
enum Action {
    /// Connects to the backend and serves its work.
    Run {
        #[command(flatten)]
        installation: Installation,
        /// A managed guest's boot record, which names the backend.
        #[arg(long, conflicts_with = "backend")]
        managed_boot: Option<PathBuf>,
        /// How the device appears to the backend; the hostname by default.
        #[arg(long, env = "DEMI_RUNNER_NAME", hide = true)]
        name: Option<String>,
        /// Marks a runner the backend manages, when set and not empty.
        #[arg(long, env = "DEMI_RUNNER_MANAGED", hide = true)]
        managed: Option<String>,
        /// The artifact cache, which runners of one user may share;
        /// `artifacts` in the installation's directory by default.
        #[arg(long, env = "DEMI_ARTIFACTS", hide = true)]
        artifacts: Option<PathBuf>,
    },
    /// Reports whether the installation's runner is active; exits with 3
    /// when it runs another release.
    Status {
        #[command(flatten)]
        installation: Installation,
    },
    /// Stops admitting work, waits for the running jobs, and releases the
    /// installation.
    Drain {
        #[command(flatten)]
        installation: Installation,
    },
    /// Removes the runner from this device: asks the backend to revoke the
    /// device, drains the runner, and removes the installation.
    Uninstall {
        #[command(flatten)]
        installation: Installation,
    },
}

/// Which installation a command acts on.
#[derive(clap::Args)]
struct Installation {
    /// The backend the installation belongs to.
    #[arg(long)]
    backend: Option<BackendUrl>,
    /// The installation's directory; one per backend under
    /// `~/.demi/instances` by default.
    #[arg(long = "home", env = "DEMI_HOME", hide = true)]
    home: Option<PathBuf>,
    /// This runner's release, which `status` compares with the active one.
    #[arg(long, env = demi_runner_protocol::release::RELEASE_ENV, hide = true)]
    release: Option<String>,
}

async fn runner(cli: Cli, shell: ShellRuntime) -> io::Result<u8> {
    let (installation, boot_path, name, managed, artifacts, removing) = match cli.action {
        Action::Run {
            installation,
            managed_boot,
            name,
            managed,
            artifacts,
        } => (installation, managed_boot, name, managed, artifacts, false),
        Action::Status { installation } => {
            let state = RunnerState::open(directory(&installation, None)?).await?;
            let release = installation.release.as_deref();
            let stdout = tokio::fs::File::from_std(standard_file(1)?);
            return manage(state, management::Action::Status, release, stdout).await;
        }
        Action::Drain { installation } => {
            let state = RunnerState::open(directory(&installation, None)?).await?;
            let release = installation.release.as_deref();
            let stdout = tokio::fs::File::from_std(standard_file(1)?);
            return manage(state, management::Action::Drain, release, stdout).await;
        }
        Action::Uninstall { installation } => {
            let directory = directory(&installation, None)?;
            let state = RunnerState::open(directory.clone()).await?;
            match state.try_lock()? {
                // The active runner asks its backend to revoke the device,
                // drains and ends; then its installation goes.
                None => {
                    let action = management::Action::Uninstall;
                    let code = manage(state, action, None, tokio::io::sink()).await?;
                    if code != 0 {
                        return Ok(code);
                    }
                    return uninstalled(&directory);
                }
                // No runner is active: this process runs one that asks the
                // backend, then removes the installation.
                Some(lease) => {
                    lease.release()?;
                    (installation, None, None, None, None, true)
                }
            }
        }
    };
    let boot = match boot_path {
        Some(path) => {
            let bytes = tokio::fs::read(path).await?;
            let boot = ManagedBoot::decode(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            Some(boot)
        }
        None => None,
    };
    let directory = directory(&installation, boot.as_ref())?;
    let backend = boot
        .as_ref()
        .map(|boot| boot.backend_url.clone())
        .or(installation.backend);
    let env: BTreeMap<_, _> = std::env::vars().collect();
    let home = home()?;
    let state = RunnerState::open(directory.clone()).await?;
    let backend = match backend {
        Some(backend) => backend,
        None => {
            state
                .config()
                .await?
                .ok_or_else(|| io::Error::other("pass --backend <url> on first start"))?
                .backend_url
        }
    };
    let executable = std::env::current_exe()?;
    // A managed guest's runner comes with its image, never from an
    // installer (`runner.md` § Runner updates).
    let installed = match &boot {
        Some(_) => None,
        None => update::Installed::of(&directory, installation.release.as_deref(), &executable),
    };
    // A managed guest is never paired, so nobody removes its runner; one that
    // `uninstall` runs is being removed.
    let removal = (boot.is_none() && !removing)
        .then(|| removal::command(&directory, installed.is_some(), &executable));
    let identity = identity(home.to_string_lossy().into_owned())?;
    let runner = wire::RunnerInfo {
        native_target: Some(host_target().into()),
        name: name.unwrap_or_else(|| identity.hostname.clone()),
        platform: if cfg!(target_os = "macos") {
            RunnerPlatform::Darwin
        } else if cfg!(windows) {
            RunnerPlatform::Win32
        } else {
            RunnerPlatform::Linux
        },
        version: installation
            .release
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
        identity,
        managed: boot
            .as_ref()
            .map(|_| true)
            .or_else(|| managed.map(|value| !value.is_empty())),
    };
    // A managed guest's state is temporary; its log and its job output stay
    // on the system layer, which a stop keeps (`runner.md` § Host log,
    // § Pipes and output).
    let (log_directory, jobs) = if boot.is_some() {
        (
            PathBuf::from("/var/log/demi"),
            PathBuf::from("/var/lib/demi/jobs"),
        )
    } else {
        (directory.join("log"), directory.join("jobs"))
    };
    let (log, layer) = host_log::open(log_directory).await?;
    tracing_subscriber::registry()
        .with(layer.with_filter(LevelFilter::INFO))
        .with(host_log::Console.with_filter(LevelFilter::WARN))
        .init();
    // Every job and stream shares this process's open files (`runner.md`
    // § Load); without the raise the runner still works, waiting sooner.
    #[cfg(unix)]
    match demi_runner_process::process::raise_open_file_limit() {
        Ok((started, raised)) => {
            tracing::info!("open-file limit {raised} (started with {started})")
        }
        Err(error) => tracing::warn!("the open-file limit could not be raised: {error}"),
    }
    // Read once before any job runs, since reading it may briefly set a
    // stricter one (`process::umask`).
    #[cfg(unix)]
    let _umask = demi_runner_process::process::umask();
    let installation_directory = directory.clone();
    let options = Options {
        backend,
        log: log.reader(),
        // A Cloud keeps its artifacts on its home image, which its stops,
        // wakes and resets keep (`native-runtime.md` § The cache).
        artifacts: artifacts.unwrap_or_else(|| match &boot {
            Some(_) => home.join(".demi/artifacts"),
            None => directory.join("artifacts"),
        }),
        directory,
        jobs,
        executable,
        cwd: std::env::current_dir()?,
        env,
        runner,
        token: boot.as_ref().map(|boot| boot.device_token.clone()),
        volumes: if boot.is_some() {
            vec![
                ManagedVolume {
                    name: wire::VolumeName::System,
                    mount: "/".into(),
                },
                ManagedVolume {
                    name: wire::VolumeName::Home,
                    mount: "/home".into(),
                },
            ]
        } else {
            vec![]
        },
        shell: Arc::new(shell),
        // One that `uninstall` runs only asks for the revocation, and never
        // updates itself.
        installed: installed.filter(|_| !removing),
        removal,
        removing,
    };
    let stop = CancellationToken::new();
    let running = registration::run(options, stop.clone());
    tokio::pin!(running);
    let outcome = tokio::select! {
        result = &mut running => result,
        result = signal() => match result {
            Ok(()) => {
                stop.cancel();
                running.await
            }
            Err(error) => Err(error),
        },
    };
    // Past this point the log holds what went wrong, and the console hears it.
    if let Err(error) = &outcome {
        tracing::error!("{error}");
    }
    log.close().await;
    // Whatever the backend answered, or when it could not be reached, the
    // installation goes.
    if removing {
        return uninstalled(&installation_directory);
    }
    match outcome {
        Ok(Ending::Stopped) => {}
        Ok(Ending::Replaced(successor)) => successor.start()?,
        Ok(Ending::Removed) => {
            eprintln!("demi-runner: this device was revoked; removing this runner");
            removal::remove(&installation_directory)?;
        }
        Err(_) => return Ok(1),
    }
    Ok(0)
}

/// Removes the installation in `directory` once its runner ended, and says
/// so.
fn uninstalled(directory: &Path) -> io::Result<u8> {
    removal::remove(directory)?;
    println!("Removed the runner of {}", directory.display());
    Ok(0)
}

fn identity(home_dir: String) -> io::Result<wire::HostIdentity> {
    let hostname = hostname::get()?
        .into_string()
        .map_err(|_| io::Error::other("the hostname is not UTF-8"))?;
    #[cfg(unix)]
    let (uid, gid) = (
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw(),
    );
    // Windows has no numeric user and group for the runner to report.
    #[cfg(windows)]
    let (uid, gid) = (0, 0);
    Ok(wire::HostIdentity {
        uid,
        gid,
        hostname,
        home_dir,
    })
}

async fn signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
        Ok(())
    }
    #[cfg(windows)]
    {
        tokio::signal::ctrl_c().await
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let name = Path::new(&args[0])
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    // A root command's alias passes its arguments through untouched.
    let cli = (name == PROGRAM).then(<Cli as clap::Parser>::parse);
    // Shutdown leaves the runtimes' remaining threads to the process exit
    // below; runner shutdown has joined its owned jobs and services already.
    let result = if let Some(cli) = cli {
        // The control thread's state belongs to one registration, so it runs
        // alone; shell jobs have a runtime of their own.
        let shell = ShellRuntime::build().expect("shell runtime");
        let control = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build_local(tokio::runtime::LocalOptions::default())
            .expect("control runtime");
        let result = control.block_on(runner(cli, ShellRuntime::new(&shell)));
        control.shutdown_background();
        shell.shutdown_background();
        result
    } else {
        // A command alias serves one invocation (`concurrency.md` § Runner)
        // and has no log of its own; the runner it calls logs.
        tracing_subscriber::registry()
            .with(host_log::Console.with_filter(LevelFilter::WARN))
            .init();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("command runtime");
        let result = runtime.block_on(command(name.into(), args[1..].to_vec()));
        runtime.shutdown_background();
        result
    };
    let code = match result {
        Ok(code) => code,
        // The reader of a command's output went away, as `head` does once it
        // has read its lines: the command ends as a program that writes into
        // a closed pipe ends in a shell, with 141 and nothing on stderr
        // (`commands.md` § Handle an rpc call).
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => 141,
        Err(error) => {
            eprintln!("demi-runner: {error}");
            1
        }
    };
    std::process::exit(i32::from(code));
}
