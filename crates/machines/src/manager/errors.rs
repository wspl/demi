//! Why an operation failed. The reply carries an error's own message, the
//! summary; the log follows its causes, which for an error that gathers
//! several failures names each of them.

use std::{fmt, io};

use demi_machines_protocol::{BaseVersion, IdError};

use super::admission::Closed;
use crate::{
    network::{NetworkError, slots::Exhausted},
    recovery::RecoveryError,
    sandbox::SandboxError,
    storage::{ext4::Ext4Error, store::StoreError, working::SaveError},
    tools::ToolError,
};

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error(transparent)]
    InvalidName(#[from] IdError),
    #[error(transparent)]
    Stopping(#[from] Closed),
    #[error("the device's worker stopped")]
    WorkerStopped,
    #[error("Cannot publish working disks with an active writer")]
    ActiveWriter,
    #[error("Cloud is not running")]
    NotRunning,
    #[error("Cloud working manifest is missing")]
    NoWorkingManifest,
    #[error("Cloud base {0} is not imported")]
    MissingBase(BaseVersion),
    #[error("Cloud start and cleanup failed; working storage retained")]
    StartAndCleanup(#[source] Parts),
    #[error("Cloud checkpoint recovery failed")]
    CheckpointRecovery(#[source] Parts),
    #[error("Cloud shutdown failed; working state retained")]
    Shutdown(#[source] Parts),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(transparent)]
    Ext4(#[from] Ext4Error),
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Slots(#[from] Exhausted),
    #[error(transparent)]
    Network(#[from] NetworkError),
    #[error(transparent)]
    Recovery(#[from] RecoveryError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Several failures of one operation, such as a failed start and the failed
/// cleanup after it, each with its causes.
#[derive(Debug)]
pub struct Parts(pub Vec<OpError>);

impl fmt::Display for Parts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for part in &self.0 {
            if !first {
                formatter.write_str("; ")?;
            }
            first = false;
            formatter.write_str(&crate::server::chain(part))?;
        }
        Ok(())
    }
}

impl std::error::Error for Parts {}
