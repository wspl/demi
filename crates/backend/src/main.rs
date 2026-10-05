//! `demi-backend`: reads its configuration, publishes its server release's
//! command packages, serves until SIGINT or SIGTERM, then shuts down in
//! order (`backend.md` § Startup and shutdown).

use std::path::Path;
use std::process::ExitCode;

use clap::{CommandFactory as _, Parser as _};
use demi_backend::{Backend, BackendConfig, Config, StartError, publish_commands};
use demi_backend_runners::native::NativeCatalog;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

fn main() -> ExitCode {
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
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(config.log.clone())
        .init();
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
    runtime.block_on(run(config))
}

async fn run(config: Config) -> ExitCode {
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
    settings.native = match publish(&settings, &release, &mut stop).await {
        Ok(native) => native,
        Err(error) => {
            eprintln!("demi-backend: {error}");
            return ExitCode::FAILURE;
        }
    };
    let data = settings.data_dir.clone();
    let backend = match Backend::start(settings).await {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("demi-backend: {error}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        address = %backend.local_addr(),
        data = %data.display(),
        mode = %config.mode,
        "demi-backend is listening"
    );
    stop.requested().await;
    match backend.close().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(errors) => {
            eprintln!("demi-backend: {errors}");
            ExitCode::FAILURE
        }
    }
}

/// Publishes the command packages of the server release whose root is
/// `release` before the backend accepts requests (`native-runtime.md`
/// § Publish packages, then source artifacts on demand); a stop signal
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
