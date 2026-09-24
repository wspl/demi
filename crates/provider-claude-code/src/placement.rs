//! The placement a runtime starts its CLI processes through
//! (`claude-code.md` § How a runtime gets its process): the backend chooses
//! the machine and readies Demi's CLI there, and the provider builds the
//! spawn request from what the placement found.

use demi_shell::{Process, SpawnRequest};
use futures_util::future::LocalBoxFuture;

/// Where a new CLI process runs on the machine the placement chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliSite {
    /// The absolute path of Demi's CLI executable there.
    pub executable: String,
    /// The process's working directory, `~/.demi/claude/run`.
    pub run_dir: String,
    /// The CLI's configuration home, `~/.demi/claude/config`.
    pub config_dir: String,
}

/// Why a CLI process could not be started: the machine could not be reached,
/// Demi's CLI could not be installed there, or the Host refused the start.
/// The message says why, with the version when an install failed, and is
/// the request's failure as it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StartError(pub String);

/// Starts the CLI processes of a runtime, on a machine the placement
/// chooses. It lives on the user's shard with the runtime.
pub trait Placement {
    /// Starts a new CLI process: the placement readies the machine and
    /// Demi's CLI there, `spawn` builds the request from the site it found,
    /// and the placement starts that request on the machine's Host. Dropping
    /// the future gives the start up.
    fn start<'a>(
        &'a self,
        spawn: &'a dyn Fn(&CliSite) -> SpawnRequest,
    ) -> LocalBoxFuture<'a, Result<Process, StartError>>;
}
