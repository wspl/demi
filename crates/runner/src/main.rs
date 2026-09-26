use demi_command_service::protocol::LocalInvocation;
use demi_runner::{
    commands::command_client::{self, RawCommand, Stdio},
    host_log, management,
    registration::{self, Options},
    state::{self, RunnerState},
    stdio::{self, standard_file},
};
use demi_runner_protocol::{boot::ManagedBoot, values::BackendUrl, wire::RunnerPlatform};
use tracing_subscriber::{
    Layer as _, filter::LevelFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

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
    let request = RawCommand::new(context, root, argv, stdio::is_live(&stdin, &env)?)?;
    let invocation = LocalInvocation {
        operation: "raw".into(),
        invocation_id: uuid::Uuid::new_v4().simple().to_string(),
        args: serde_json::to_value(request).map_err(io::Error::other)?,
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        env,
    };
    let stop = CancellationToken::new();
    tokio::select! {
        result = command_client::forward(&endpoint, &invocation, Stdio {
            stdin: tokio::fs::File::from_std(stdin),
            stdout: tokio::fs::File::from_std(standard_file(1)?),
            stderr: tokio::fs::File::from_std(standard_file(2)?),
        }, stop.clone()) => result.map(|result| result.exit_code),
        result = signal() => {
            result?;
            stop.cancel();
            Ok(130)
        },
    }
}

/// Asks the installation's active runner for its status or to drain.
async fn manage(
    state: RunnerState,
    action: management::Action,
    release: Option<&str>,
) -> io::Result<u8> {
    let active = state.active().await?;
    let request = LocalInvocation {
        operation: "manage".into(),
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
            stdout: tokio::fs::File::from_std(standard_file(1)?),
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
        management::Action::Drain => loop {
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

/// Runs this device as a Demi execution target.
#[derive(clap::Parser)]
#[command(name = "demi-runner", version)]
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
    #[arg(long, env = "DEMI_RELEASE_ID", hide = true)]
    release: Option<String>,
}

async fn runner(cli: Cli, shell: demi_runner::shell::ShellRuntime) -> io::Result<u8> {
    let (installation, boot_path, name, managed) = match cli.action {
        Action::Run {
            installation,
            managed_boot,
            name,
            managed,
        } => (installation, managed_boot, name, managed),
        Action::Status { installation } => {
            let state = RunnerState::open(directory(&installation, None)?).await?;
            let release = installation.release.as_deref();
            return manage(state, management::Action::Status, release).await;
        }
        Action::Drain { installation } => {
            let state = RunnerState::open(directory(&installation, None)?).await?;
            let release = installation.release.as_deref();
            return manage(state, management::Action::Drain, release).await;
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
    let identity = identity(home.to_string_lossy().into_owned())?;
    let runner = demi_runner::connection::wire::RunnerInfo {
        native_target: Some(demi_runner::services::target().into()),
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
    // A managed guest's state is temporary; its log stays on the system
    // layer (`runner.md` § Host log).
    let log_directory = if boot.is_some() {
        PathBuf::from("/var/log/demi")
    } else {
        directory.join("log")
    };
    let (log, layer) = host_log::open(log_directory).await?;
    tracing_subscriber::registry()
        .with(layer.with_filter(LevelFilter::INFO))
        .with(host_log::Console.with_filter(LevelFilter::WARN))
        .init();
    // Every job and stream shares this process's open files (`runner.md`
    // § Load); without the raise the runner still works, waiting sooner.
    #[cfg(unix)]
    match demi_runner::process::raise_open_file_limit() {
        Ok((started, raised)) => {
            tracing::info!("open-file limit {raised} (started with {started})")
        }
        Err(error) => tracing::warn!("the open-file limit could not be raised: {error}"),
    }
    // Read once before any job runs, since reading it may briefly set a
    // stricter one (`process::umask`).
    #[cfg(unix)]
    let _umask = demi_runner::process::umask();
    let options = Options {
        backend,
        log: log.reader(),
        directory,
        executable: std::env::current_exe()?,
        cwd: std::env::current_dir()?,
        env,
        runner,
        token: boot.as_ref().map(|boot| boot.device_token.clone()),
        volumes: if boot.is_some() {
            vec![
                demi_runner::volumes::ManagedVolume {
                    name: demi_runner::connection::wire::VolumeName::System,
                    mount: "/".into(),
                },
                demi_runner::volumes::ManagedVolume {
                    name: demi_runner::connection::wire::VolumeName::Home,
                    mount: "/home".into(),
                },
            ]
        } else {
            vec![]
        },
        shell,
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
    let code = match outcome {
        Ok(()) => 0,
        Err(error) => {
            tracing::error!("{error}");
            1
        }
    };
    log.close().await;
    Ok(code)
}

fn identity(home_dir: String) -> io::Result<demi_runner::connection::wire::HostIdentity> {
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
    Ok(demi_runner::connection::wire::HostIdentity {
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
    let cli = (name == "demi-runner").then(<Cli as clap::Parser>::parse);
    // Shutdown leaves the runtimes' remaining threads to the process exit
    // below; runner shutdown has joined its owned jobs and services already.
    let result = if let Some(cli) = cli {
        // The control thread's state belongs to one registration, so it runs
        // alone; shell jobs have a runtime of their own.
        let shell = demi_runner::shell::ShellRuntime::build().expect("shell runtime");
        let control = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build_local(tokio::runtime::LocalOptions::default())
            .expect("control runtime");
        let result = control.block_on(runner(cli, demi_runner::shell::ShellRuntime::new(&shell)));
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
        Err(error) => {
            eprintln!("demi-runner: {error}");
            1
        }
    };
    std::process::exit(i32::from(code));
}
