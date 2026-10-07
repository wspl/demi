//! How a runner starts a command program, and how the program reads it
//! (`native-runtime.md` § Invoke and retire a service): with
//! `--command-service`, and `--data` naming the package's own directory in
//! the runner instance's state.

use std::path::{Path, PathBuf};

use clap::Parser;

/// The argument a runner starts a command program with.
pub const COMMAND_SERVICE: &str = "--command-service";
/// The argument naming the package's data directory, which follows it.
pub const DATA: &str = "--data";

/// What a command program was started with.
#[derive(Debug, Parser)]
#[command(disable_help_flag = true, disable_version_flag = true)]
pub struct Launch {
    /// Serves the command protocol over standard input and output.
    #[arg(long = "command-service", required = true)]
    _command_service: bool,
    /// Where the package keeps what outlives it on this Host: a directory of
    /// its own in the state of the runner instance that started it, which
    /// may not exist yet. Two runner instances on one machine name two.
    #[arg(long = "data")]
    data: Option<PathBuf>,
}

impl Launch {
    /// The process's arguments; anything else prints the usage and exits
    /// with status 2.
    pub fn from_process() -> Self {
        Self::parse()
    }

    /// The package's data directory; none when the runner named none.
    pub fn data(&self) -> Option<&Path> {
        self.data.as_deref()
    }
}
