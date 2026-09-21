use demi_command_service::protocol::LocalInvocation;
use demi_runner::{
    commands::command_client::{self, RawCommand, Stdio},
    mode::{self, Options},
    state::{self, RunnerState},
    stdio::{self, standard_file},
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

async fn runner(args: Vec<String>) -> io::Result<u8> {
    let action = args.first().map(String::as_str).unwrap_or("");
    if !matches!(action, "run" | "status" | "drain") {
        return Err(usage());
    }
    let backend = match &args[1..] {
        [] => None,
        [flag, value] if flag == "--backend" => Some(value.clone()),
        _ => return Err(usage()),
    };
    let env: BTreeMap<_, _> = std::env::vars().collect();
    let home = env
        .get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or_else(|| io::Error::other("user home is not configured"))?
        .clone();
    let directory = if let Some(directory) = env.get("DEMI_HOME") {
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
    let runner = demi_runner::connection::wire::HelloRunner {
        native_target: Some(demi_runner::commands::native::target().into()),
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
        managed: env
            .get("DEMI_RUNNER_MANAGED")
            .map(|value| !value.is_empty()),
    };
    let options = Options {
        backend,
        log: directory.join("log"),
        directory,
        executable: std::env::current_exe()?,
        cwd: std::env::current_dir()?,
        env,
        runner,
        token: None,
        volumes: vec![],
    };
    let stop = CancellationToken::new();
    let running = mode::run(options, stop.clone());
    tokio::pin!(running);
    tokio::select! {
        result = &mut running => result?,
        result = signal() => {
            result?;
            stop.cancel();
            running.await?;
        },
    }
    Ok(0)
}

fn identity(home_dir: String) -> io::Result<demi_runner::connection::wire::HelloRunnerIdentity> {
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
        Ok(demi_runner::connection::wire::HelloRunnerIdentity {
            uid: unsafe { libc::getuid() }.into(),
            gid: unsafe { libc::getgid() }.into(),
            hostname: String::from_utf8(hostname[..length].to_vec()).map_err(io::Error::other)?,
            home_dir,
        })
    }
    #[cfg(windows)]
    {
        Ok(demi_runner::connection::wire::HelloRunnerIdentity {
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
        "Usage: demi-runner <run|status|drain> [--backend <url>]",
    )
}

fn main() {
    #[cfg(target_os = "linux")]
    if std::process::id() == 1 {
        let result = (|| {
            let boot = demi_runner::init::boot()?;
            if let Some(code) = demi_runner::init::supervise()? {
                return Ok(code);
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            let result = runtime.block_on(async {
                let stop = CancellationToken::new();
                let running = mode::run(boot.options, stop.clone());
                tokio::pin!(running);
                tokio::select! {
                    result = &mut running => result,
                    result = signal() => {
                        result?;
                        stop.cancel();
                        running.await
                    },
                }
            });
            runtime.shutdown_background();
            result.map(|()| 0)
        })();
        let code = match result {
            Ok(code) => code,
            Err(error) => {
                eprintln!("demi-runner: guest boot failed: {error}");
                1
            }
        };
        std::process::exit(i32::from(code));
    }
    let args: Vec<_> = std::env::args().collect();
    let name = Path::new(&args[0])
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runner runtime");
    let result = if name != "demi-runner" {
        runtime.block_on(command(name.into(), args[1..].to_vec()))
    } else {
        runtime.block_on(runner(args[1..].to_vec()))
    };
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("demi-runner: {error}");
            1
        }
    };
    // Runner shutdown has joined its owned jobs and services.
    runtime.shutdown_background();
    std::process::exit(i32::from(code));
}
