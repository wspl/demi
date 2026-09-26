//! The repository's development commands (`crates-and-packages.md`
//! § xtask). `cargo xtask` runs them; `bun run contracts` builds the
//! workspace and runs `target/debug/xtask contracts`.

mod browser;
mod cloud_image;
mod contracts;
mod native;

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
    /// Generates the browser's TypeScript contracts from the Rust contract types.
    Contracts,
    /// Builds the native executables and packages their releases.
    #[command(subcommand)]
    Native(native::Command),
    /// Pins a Chrome for Testing version: writes its release record.
    BrowserRelease(browser::Options),
    /// Completes a Cloud image release on its Linux builder.
    #[command(subcommand)]
    CloudImage(cloud_image::Command),
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
/// cancels, so a publication it makes stops and leaves nothing behind.
fn interruptible<T, F>(work: impl FnOnce(CancellationToken) -> F) -> std::io::Result<T>
where
    F: Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    Ok(runtime.block_on(async {
        let cancel = CancellationToken::new();
        let interrupt = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    cancel.cancel();
                }
            }
        });
        let result = work(cancel).await;
        interrupt.abort();
        result
    }))
}

fn main() -> ExitCode {
    match Cli::parse().command {
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
    }
}
