//! `demi-core`: the process a program runs Demi's cores in
//! (`crates/core-api/src/process/mod.rs`).
//!
//! The program starts it and speaks HTTP/2 to it over its standard input
//! and output. It logs to its standard error, and ends once the program's
//! connection does.
//!
//! `DEMI_PUBLIC_URL` says where the program's server forwards the process's
//! routes; without it, the cores offer neither a plugin's OAuth nor a
//! schedule's URL.

use std::env::VarError;
use std::process::ExitCode;

use url::Url;

fn main() -> ExitCode {
    // A panic leaves its report on standard error.
    demi_shared_cli::install_panic_hook();
    // Standard output carries the program's connection, and nothing else.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();
    let public_url = match std::env::var("DEMI_PUBLIC_URL") {
        Ok(text) => match text.parse::<Url>() {
            Ok(url) => Some(url),
            Err(error) => {
                tracing::error!(event = "process.public_url_unusable", %error, "DEMI_PUBLIC_URL is no URL");
                return ExitCode::FAILURE;
            }
        },
        Err(VarError::NotPresent) => None,
        Err(error) => {
            tracing::error!(event = "process.public_url_unusable", %error, "DEMI_PUBLIC_URL is not readable");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(event = "process.runtime_failed", %error, "the runtime did not start");
            return ExitCode::FAILURE;
        }
    };
    let connection = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    match runtime.block_on(demi_core_api::process::run(connection, public_url)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(
                event = "process.failed",
                error = &error as &dyn std::error::Error,
                "the process did not run"
            );
            ExitCode::FAILURE
        }
    }
}
