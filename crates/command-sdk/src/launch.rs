//! How a runner starts a command program, and how the program reads it
//! (`native-runtime.md` § Invoke and retire a service): with
//! `--command-service` alone.

use clap::Parser;

/// The one argument a runner starts a command program with.
pub const COMMAND_SERVICE: &str = "--command-service";

/// What a command program was started with.
#[derive(Debug, Parser)]
#[command(disable_help_flag = true, disable_version_flag = true)]
pub struct Launch {
    /// Serves the command protocol over standard input and output.
    #[arg(long = "command-service", required = true)]
    _command_service: bool,
}

impl Launch {
    /// The process's arguments; anything else prints the usage and exits
    /// with status 2.
    pub fn from_process() -> Self {
        Self::parse()
    }
}
