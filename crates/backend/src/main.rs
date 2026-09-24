//! `demi-backend`: reads its configuration, serves until SIGINT or SIGTERM,
//! then shuts down in order (`backend.md` § Startup and shutdown).

use std::path::Path;
use std::process::ExitCode;

use clap::Parser as _;
use demi_backend::{Backend, Config, NativeCatalog, PublicationError, publish_native};
use tokio_util::sync::CancellationToken;

fn main() -> ExitCode {
    // An unusable value stops here, naming its variable.
    let config = Config::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("demi-backend: the runtime cannot start: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(config))
}

async fn run(config: Config) -> ExitCode {
    let mut settings = match config.backend() {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("demi-backend: {error}");
            return ExitCode::FAILURE;
        }
    };
    settings.native = match publish(&config.native_config).await {
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
    if let Err(error) = stopped().await {
        tracing::error!(error = &error as &dyn std::error::Error, "the stop signals cannot be watched");
    }
    match backend.close().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(errors) => {
            eprintln!("demi-backend: {errors}");
            ExitCode::FAILURE
        }
    }
}

/// Publishes the native releases before the backend accepts requests
/// (`native-runtime.md` § Publish artifacts before enabling commands); a
/// stop signal interrupts it.
async fn publish(path: &Path) -> Result<NativeCatalog, PublicationError> {
    let cancel = CancellationToken::new();
    let publication = publish_native(path, &cancel);
    tokio::pin!(publication);
    tokio::select! {
        published = &mut publication => return published,
        // Signals that cannot be watched leave the publication to finish.
        Ok(()) = stopped() => {}
    }
    cancel.cancel();
    publication.await
}

/// Waits for SIGINT or SIGTERM.
async fn stopped() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        interrupted = tokio::signal::ctrl_c() => interrupted,
        _ = terminate.recv() => Ok(()),
    }
}
