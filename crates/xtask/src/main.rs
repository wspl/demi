//! The repository's development commands (`crates-and-packages.md`
//! § xtask). `bun xtask` builds the workspace and runs them
//! (`builds-and-releases.md`).

#[cfg(test)]
mod boundaries;
mod browser;
mod cloud_image;
// Generating the contracts reads the backend's plugins, whose library builds
// only where the backend runs; on Windows xtask builds the native releases.
#[cfg(all(unix, feature = "developer"))]
mod contracts;
#[cfg(all(unix, feature = "developer"))]
mod dev;
mod native;
mod preview_runtime;
mod server_release;
// Reads Unix file metadata: the sweep runs where development builds do.
#[cfg(unix)]
mod sweep;
mod vendor;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "xtask", about = "The repository's development commands")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generates the web app's TypeScript contracts from the Rust contract types.
    #[cfg(all(unix, feature = "developer"))]
    Contracts,
    /// Builds the native executables and packages their releases.
    #[command(subcommand)]
    Native(native::Command),
    /// Builds the web preview's runtime with the rewriter's WebAssembly.
    PreviewRuntime(preview_runtime::Options),
    /// Assembles a server release root from the built executables.
    ServerRelease(server_release::Options),
    /// Pins a Chrome for Testing version: writes its release record.
    BrowserRelease(browser::Options),
    /// Completes a Cloud image release on its Linux builder.
    #[command(subcommand)]
    CloudImage(cloud_image::Command),
    /// Compares the vendored crates with their upstream releases.
    #[command(subcommand)]
    Vendor(vendor::Command),
    /// Removes the build products no current build of the checkout uses.
    #[cfg(unix)]
    Sweep(sweep::Options),
    /// Runs a development backend with a scripted Cloud and the models `.env` turns on.
    #[cfg(all(unix, feature = "developer"))]
    Dev(dev::Options),
}

/// The repository's root directory.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask sits two directories below the repository's root")
        .to_owned()
}

/// A release record's file: indented JSON with a final newline.
fn record(value: &impl serde::Serialize) -> serde_json::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Runs `work` on a runtime of this thread with a token that an interrupt
/// (or, on Unix, a termination or a hang-up) cancels, so a publication it
/// makes stops and leaves nothing behind, and the processes `xtask dev`
/// started are stopped.
fn interruptible<T, F>(work: impl FnOnce(CancellationToken) -> F) -> std::io::Result<T>
where
    F: Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // The handlers are in place before `work` starts anything.
        let stop = stop_requested()?;
        let cancel = CancellationToken::new();
        let interrupt = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                stop.await;
                cancel.cancel();
            }
        });
        let result = work(cancel).await;
        interrupt.abort();
        Ok(result)
    })
}

/// A future that ends when the user interrupts the command, or when it is
/// terminated or its terminal hangs up; the handlers it installs replace the
/// signals' default of ending the process at once.
#[cfg(unix)]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    })
}

/// A future that ends when the user interrupts the command.
#[cfg(not(unix))]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    Ok(async {
        // Without a handler, nothing interrupts the work; it runs to its end.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    })
}

fn main() -> ExitCode {
    match Cli::parse().command {
        #[cfg(all(unix, feature = "developer"))]
        Command::Contracts => match contracts::run() {
            Ok(written) => {
                for path in written {
                    println!("wrote {}", path.display());
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("xtask contracts: {error}");
                ExitCode::FAILURE
            }
        },
        Command::Native(command) => match native::run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask native: {error}");
                ExitCode::FAILURE
            }
        },
        Command::PreviewRuntime(options) => match preview_runtime::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask preview-runtime: {error}");
                ExitCode::FAILURE
            }
        },
        Command::ServerRelease(options) => match server_release::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask server-release: {error}");
                ExitCode::FAILURE
            }
        },
        Command::BrowserRelease(options) => match browser::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask browser-release: {error}");
                ExitCode::FAILURE
            }
        },
        Command::CloudImage(command) => match cloud_image::run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask cloud-image: {error}");
                ExitCode::FAILURE
            }
        },
        Command::Vendor(command) => match vendor::run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask vendor: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(unix)]
        Command::Sweep(options) => match sweep::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask sweep: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(all(unix, feature = "developer"))]
        Command::Dev(options) => match dev::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask dev: {error}");
                ExitCode::FAILURE
            }
        },
    }
}
