//! The repository's development commands (`crates-and-packages.md`
//! § xtask). `cargo xtask` runs them; `bun run contracts` builds the
//! workspace and runs `target/debug/xtask contracts`.

mod contracts;
mod native;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
}

/// The repository's root directory.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask sits two directories below the repository's root")
        .to_owned()
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
    }
}
