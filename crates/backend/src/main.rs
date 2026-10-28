//! `demi-backend`: the executable around the library.
//!
//! It reads its configuration, publishes its server release's command
//! packages, serves until SIGINT or SIGTERM, then shuts down in order
//! (`Backend::close`).
//!
//! The stop signals are watched from the first step:
//!
//! - A signal that comes while the command packages are being published stops
//!   the start.
//! - A signal that comes later is kept: the start finishes and the shutdown
//!   follows at once.
//!
//! Readiness goes to systemd, whose unit waits for it without a timeout,
//! since a migration or an image import can take minutes.

use std::path::Path;
use std::process::ExitCode;

use clap::{CommandFactory as _, Parser as _};
use demi_backend::{Backend, BackendConfig, Config, StartError, publish_commands};
use demi_backend_database::server_log;
use demi_backend_runners::native::NativeCatalog;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

fn main() -> ExitCode {
    // A panic leaves its report on standard error.
    demi_shared_cli::install_panic_hook();
    // Every socket, database and stream shares one table of open files, and
    // systemd starts the backend with a soft limit of 1024 under a hard one
    // of 524288; the outcome is logged once the log is up.
    #[cfg(unix)]
    let open_files = demi_shared_cli::raise_open_file_limit();
    // An unusable value stops here, naming its variable, and so does a
    // variable no setting reads, such as a misspelt one. The machine
    // manager's own settings, which share the configuration file, are its
    // to check.
    let config = Config::parse();
    let variables = std::env::vars_os().filter(|(name, _)| {
        !name
            .to_string_lossy()
            .starts_with(demi_shared_cli::MANAGED_PREFIX)
    });
    if let Some(name) = demi_shared_cli::unknown_variable(&Config::command(), "DEMI_", variables)
    {
        eprintln!(
            "demi-backend: {name} is not a backend setting; `demi-backend --help` lists them"
        );
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("demi-backend: the runtime cannot start: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(
        config,
        #[cfg(unix)]
        open_files,
    ))
}

async fn run(
    config: Config,
    #[cfg(unix)] open_files: std::io::Result<demi_shared_cli::OpenFileLimit>,
) -> ExitCode {
    let started = std::time::Instant::now();
    // Installed before anything starts, so a stop signal that comes while the
    // backend starts is kept and ends it once it is up.
    let mut stop = match StopSignals::install() {
        Ok(stop) => stop,
        Err(error) => {
            eprintln!("demi-backend: the stop signals cannot be watched: {error}");
            return ExitCode::FAILURE;
        }
    };
    let release = match config.release() {
        Ok(release) => release,
        Err(error) => {
            eprintln!("demi-backend: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut settings = match config.backend(&release) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("demi-backend: {error}");
            return ExitCode::FAILURE;
        }
    };
    // An upgrade checks the next release's view of the configuration before
    // it stops anything.
    if config.check_config {
        return ExitCode::SUCCESS;
    }
    if let Some(record) = &config.update_screen {
        return update_screen(&config, &settings, record, &mut stop).await;
    }
    // What `DEMI_LOG` selects goes to standard error, which the process
    // supervisor keeps, and to the server log in the data directory.
    let limits = server_log::Limits {
        keep: std::time::Duration::from_secs(u64::from(config.log_keep_days) * 24 * 60 * 60),
        max_bytes: config.log_max_size,
    };
    let (log, log_layer) = match server_log::open(&settings.data_dir, limits, config.log.clone()) {
        Ok(log) => log,
        Err(error) => {
            eprintln!("demi-backend: the server log cannot start: {error}");
            return ExitCode::FAILURE;
        }
    };
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(log_layer)
        .with(config.log.clone())
        .init();
    settings.log = log.log();
    // A refused raise leaves the limit as it was; the backend still works,
    // running out of files sooner.
    #[cfg(unix)]
    let open_files_limit = match open_files {
        Ok(limit) => {
            tracing::info!(
                event = "open_files.limit_raised",
                raised = limit.raised,
                started_soft = limit.started_soft,
                "the open-file limit was raised"
            );
            Some(limit.raised)
        }
        Err(error) => {
            tracing::warn!(event = "open_files.limit_raise_failed", %error, "the open-file limit could not be raised");
            None
        }
    };
    let served = serve(settings, &release, &mut stop, started, open_files_limit).await;
    log.close().await;
    served
}

/// Answers every page with the update screen of the steps in `record` until
/// a stop signal: no storage opens, and the log goes to standard error only,
/// since the data directory's log is the backend's.
async fn update_screen(config: &Config, settings: &BackendConfig, record: &Path, stop: &mut StopSignals) -> ExitCode {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(config.log.clone())
        .init();
    let update = std::sync::Arc::new(demi_backend_database::update::Update::screen(record.to_owned()));
    let listening = demi_backend_http::Edge::listen(
        settings.address,
        settings.public_url.as_ref(),
        settings.web_directory.clone(),
        update,
    )
    .await;
    let _listening = match listening {
        Ok(listening) => listening,
        Err(error) => {
            tracing::error!(event = "update_screen.listen_failed", address = %settings.address, %error, "the update screen did not start");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(event = "update_screen.started", address = %settings.address, "the update screen is listening");
    stop.requested().await;
    ExitCode::SUCCESS
}

/// Starts the backend and serves until a stop signal, then shuts it down.
async fn serve(
    mut settings: BackendConfig,
    release: &Path,
    stop: &mut StopSignals,
    started: std::time::Instant,
    open_files_limit: Option<u64>,
) -> ExitCode {
    settings.native = match publish(&settings, release, stop).await {
        Ok(native) => native,
        Err(error) => {
            tracing::error!(event = "backend.start_failed", step = "publish", %error, "the backend did not start");
            return ExitCode::FAILURE;
        }
    };
    let data = settings.data_dir.clone();
    let previous_exit = exit_state::begin(&data);
    let backend = match Backend::start(settings).await {
        Ok(backend) => backend,
        Err(error) => {
            tracing::error!(event = "backend.start_failed", step = "start", %error, "the backend did not start");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        event = "backend.started",
        version = env!("CARGO_PKG_VERSION"),
        mode = %backend.instance_mode(),
        address = %backend.local_addr(),
        data_dir = %data.display(),
        previous_exit,
        open_files_limit,
        ms_to_ready = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "demi-backend is listening"
    );
    // Readiness for systemd's Type=notify, which an upgrade waits for;
    // without a notify socket, as in development, this does nothing.
    if let Err(error) = sd_notify::notify(&[sd_notify::NotifyState::Ready]) {
        tracing::warn!(event = "systemd.ready_failed", %error, "systemd was not told that the backend is ready");
    }
    stop.requested().await;
    // A stop is a SIGINT or a SIGTERM, which an upgrade's restart sends as a
    // stop does: the backend cannot tell them apart.
    let census = backend.census().await;
    tracing::info!(
        event = "backend.stopping",
        cause = "signal",
        live_conversations = census.live_trees,
        running_turns = census.running_turns,
        trees_running_commands = census.trees_running_commands,
        "the backend is stopping"
    );
    match backend.close().await {
        Ok(()) => {
            exit_state::end(&data);
            ExitCode::SUCCESS
        }
        Err(errors) => {
            tracing::error!(event = "backend.close_failed", error = %errors, "the backend did not close cleanly");
            ExitCode::FAILURE
        }
    }
}

/// What the previous run left of how it ended: `running` while the backend
/// serves, `clean` once it closed. A start that finds `running` follows a run
/// the system or an operator killed.
mod exit_state {
    use std::path::Path;

    const FILE: &str = "exit-state";

    /// Answers how the previous run ended (`clean`, `killed` or `unknown`
    /// for a data directory that has no run before) and marks this run as
    /// running.
    pub(super) fn begin(data: &Path) -> &'static str {
        let path = data.join(FILE);
        let previous = match std::fs::read_to_string(&path).as_deref().map(str::trim) {
            Ok("clean") => "clean",
            Ok("running") => "killed",
            _ => "unknown",
        };
        // A directory that cannot be written leaves the next start with
        // `unknown`, and the backend serves all the same.
        let _ = std::fs::write(&path, "running");
        previous
    }

    /// Marks this run as ended in order.
    pub(super) fn end(data: &Path) {
        let _ = std::fs::write(data.join(FILE), "clean");
    }
}

/// Publishes the command packages of the server release whose root is
/// `release` before the backend accepts requests (`crates/backend-runners/src/publication.rs`); a stop signal
/// interrupts it.
async fn publish(
    settings: &BackendConfig,
    release: &Path,
    stop: &mut StopSignals,
) -> Result<NativeCatalog, StartError> {
    let cancel = CancellationToken::new();
    let publication = publish_commands(settings, release, &cancel);
    tokio::pin!(publication);
    tokio::select! {
        published = &mut publication => return published,
        () = stop.requested() => {}
    }
    cancel.cancel();
    publication.await
}

/// SIGINT and SIGTERM, watched from when they are installed: a signal that
/// comes before anything waits for it is kept until something does.
struct StopSignals {
    interrupt: Signal,
    terminate: Signal,
}

impl StopSignals {
    fn install() -> std::io::Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    /// Waits for either signal.
    async fn requested(&mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
    }
}
