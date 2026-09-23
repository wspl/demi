use demi_command_service::protocol::LocalInvocation;
use demi_runner::{
    commands::command_client::{self, RawCommand, Stdio},
    host_log,
    mode::{self, Options},
    state::{self, RunnerState},
    stdio::{self, standard_file},
};
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
    let request = RawCommand {
        context,
        root,
        argv,
        live: stdio::is_live(&stdin, &env)?,
    };
    request.validate()?;
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

async fn manage(state: RunnerState, action: &str) -> io::Result<u8> {
    let active = state.active().await?;
    let request = LocalInvocation {
        operation: "manage".into(),
        invocation_id: uuid::Uuid::new_v4().simple().to_string(),
        args: serde_json::json!({"secret": active.secret, "action": action}),
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
    if action == "drain" {
        loop {
            match state.try_lock()? {
                Some(lease) => {
                    lease.release()?;
                    break;
                }
                None => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
    }
    if action == "status"
        && std::env::var("DEMI_RELEASE_ID").is_ok_and(|release| release != active.release)
    {
        return Ok(3);
    }
    Ok(0)
}

async fn runner(args: Vec<String>, shell: demi_runner::shell::ShellRuntime) -> io::Result<u8> {
    let action = args.first().map(String::as_str).unwrap_or("");
    if !matches!(action, "run" | "status" | "drain") {
        return Err(usage());
    }
    let (backend, boot_path) = match &args[1..] {
        [] => (None, None),
        [flag, value] if flag == "--backend" => (Some(value.clone()), None),
        [flag, value] if action == "run" && flag == "--managed-boot" => (None, Some(value)),
        _ => return Err(usage()),
    };
    let boot = match boot_path {
        Some(path) => {
            let bytes = tokio::fs::read(path).await?;
            let boot = demi_runner_protocol::boot::ManagedBoot::decode(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            state::backend_url(&boot.backend_url)?;
            Some(boot)
        }
        None => None,
    };
    let backend = boot
        .as_ref()
        .map(|boot| boot.backend_url.clone())
        .or(backend);
    let env: BTreeMap<_, _> = std::env::vars().collect();
    let home = env
        .get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or_else(|| io::Error::other("user home is not configured"))?
        .clone();
    let directory = if boot.is_some() {
        PathBuf::from("/run/demi")
    } else if let Some(directory) = env.get("DEMI_HOME") {
        PathBuf::from(directory)
    } else {
        let backend = backend
            .as_deref()
            .ok_or_else(|| io::Error::other("pass --backend <url> to select an installation"))?;
        PathBuf::from(&home)
            .join(".demi/instances")
            .join(state::instance_id(backend)?)
    };
    let state = RunnerState::open(directory.clone()).await?;
    if action != "run" {
        return manage(state, action).await;
    }
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
    let identity = identity(home)?;
    let runner = demi_runner::connection::wire::RunnerInfo {
        native_target: Some(demi_runner::services::target().into()),
        name: env
            .get("DEMI_RUNNER_NAME")
            .cloned()
            .unwrap_or_else(|| identity.hostname.clone()),
        platform: if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(windows) {
            "win32"
        } else {
            "linux"
        }
        .into(),
        version: env
            .get("DEMI_RELEASE_ID")
            .cloned()
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
        identity,
        managed: boot.as_ref().map(|_| true).or_else(|| {
            env.get("DEMI_RUNNER_MANAGED")
                .map(|value| !value.is_empty())
        }),
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
    let running = mode::run(options, stop.clone());
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
    #[cfg(unix)]
    {
        let mut hostname = [0_u8; 256];
        if unsafe { libc::gethostname(hostname.as_mut_ptr().cast(), hostname.len()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let length = hostname
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| io::Error::other("hostname exceeds 255 bytes"))?;
        Ok(demi_runner::connection::wire::HostIdentity {
            uid: unsafe { libc::getuid() }.into(),
            gid: unsafe { libc::getgid() }.into(),
            hostname: String::from_utf8(hostname[..length].to_vec()).map_err(io::Error::other)?,
            home_dir,
        })
    }
    #[cfg(windows)]
    {
        Ok(demi_runner::connection::wire::HostIdentity {
            uid: 0.0,
            gid: 0.0,
            hostname: std::env::var("COMPUTERNAME").map_err(io::Error::other)?,
            home_dir,
        })
    }
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

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Usage: demi-runner <run|status|drain> [--backend <url> | --managed-boot <path>]",
    )
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let name = Path::new(&args[0])
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    // Shutdown leaves the runtimes' remaining threads to the process exit
    // below; runner shutdown has joined its owned jobs and services already.
    let result = if name != "demi-runner" {
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
    } else {
        // The control thread's state belongs to one registration, so it runs
        // alone; shell jobs have a runtime of their own.
        let shell = demi_runner::shell::ShellRuntime::build().expect("shell runtime");
        let control = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build_local(tokio::runtime::LocalOptions::default())
            .expect("control runtime");
        let result = control.block_on(runner(
            args[1..].to_vec(),
            demi_runner::shell::ShellRuntime::new(&shell),
        ));
        control.shutdown_background();
        shell.shutdown_background();
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
