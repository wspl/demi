//! A directory as its Host names it
//! (`crates/backend-database/src/selection.rs`).
//!
//! Every move asks the destination's Host one question, which follows links,
//! drops a trailing separator, spells the directory as its file system keeps it
//! and checks that it is a directory. So two spellings of one directory select
//! the one project, and a home reached through a link is the home.
//!
//! A Host that does not answer within [`QUESTION_DEADLINE`] refuses the move,
//! which then changes nothing, rather than holding it until the connection's
//! liveness gives up on the runner. Waking a stopped Cloud is not the
//! question, and takes as long as it takes.

use std::time::Duration;

use demi_backend_database::devices::DeviceRecord;
use demi_backend_remote_host::host_paths;
use demi_host_interface::{HostErrorKind, HostFs};
use demi_runner_protocol::files::FsFailure;
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::DeviceId;

use crate::HostShard;
use crate::transition::ChangeRefusal;

/// How long a Host has to answer what a directory is.
pub const QUESTION_DEADLINE: Duration = Duration::from_secs(10);

/// Why a Host named no directory for a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryRefusal {
    /// The Host is not reachable now; the text says why.
    Offline(String),
    /// Nothing is there, or it is not a directory.
    NotADirectory,
    /// The Host failed otherwise; the text is its.
    Failed(String),
    /// The Host did not answer within [`QUESTION_DEADLINE`].
    Unanswered,
}

impl DirectoryRefusal {
    /// The refusal of a move to `directory` on the device `host`.
    pub fn of_move(self, directory: &str, host: &str) -> ChangeRefusal {
        match self {
            Self::Offline(reason) => ChangeRefusal::DeviceOffline(reason),
            Self::NotADirectory => ChangeRefusal::NotADirectory(format!("{directory} is not a directory on {host}")),
            Self::Failed(reason) => ChangeRefusal::HostFailed(format!("{directory} on {host}: {reason}")),
            Self::Unanswered => ChangeRefusal::HostFailed(format!(
                "{host} did not answer within {} seconds whether {directory} is a directory there",
                QUESTION_DEADLINE.as_secs()
            )),
        }
    }
}

impl dyn HostShard + '_ {
    /// `directory` on `device` as its Host names it, which must be a
    /// directory: through device access for a paired device, whose runner
    /// must be connected, and machine access for the Cloud, which wakes it
    /// (§ Every way to a Host).
    pub async fn canonical_directory(
        &self,
        device: &DeviceRecord,
        directory: &str,
    ) -> Result<String, DirectoryRefusal> {
        let resolved = match device.kind {
            DeviceKind::Managed => {
                let access = self
                    .cloud_shard()
                    .machine_access()
                    .await
                    .map_err(|error| DirectoryRefusal::Failed(format!("Cloud: {error}")))?;
                asked(access.host.directory(directory)).await
            }
            DeviceKind::User => {
                let Some(access) = self.devices().device_access(&device.id).await else {
                    return Err(DirectoryRefusal::Offline(self.offline(device).message));
                };
                asked(access.directory(directory)).await
            }
        };
        let Some(resolved) = resolved else {
            return Err(DirectoryRefusal::Unanswered);
        };
        resolved.map_err(|error| match &error.kind {
            HostErrorKind::Failed { code }
                if matches!(FsFailure::of(code.as_deref()), FsFailure::NotFound)
                    || code.as_deref() == Some("ENOTDIR") =>
            {
                DirectoryRefusal::NotADirectory
            }
            HostErrorKind::Offline => DirectoryRefusal::Offline(self.offline(device).message),
            _ => DirectoryRefusal::Failed(error.message),
        })
    }

    /// `to` with its directory as the Host names it, as a new conversation
    /// starts there: a directory of a paired device is resolved by the
    /// device's runner. The Cloud's directory of a conversation, a Fork's
    /// source's or a Cloud project's is one the backend made and names as it
    /// is, and a project's is its own.
    pub async fn resolved_destination(
        &self,
        to: ConversationTarget,
    ) -> Result<ConversationTarget, ChangeRefusal> {
        let ConversationTarget::Device { device_id, path } = to else {
            return Ok(to);
        };
        let path = self.device_directory(&device_id, &path).await?;
        Ok(ConversationTarget::Device { device_id, path })
    }

    /// `path` on the user's paired device `device_id` as the device's
    /// runner names it.
    pub async fn device_directory(&self, device_id: &DeviceId, path: &str) -> Result<String, ChangeRefusal> {
        let device = self.paired_device(device_id).await?;
        self.canonical_directory(&device, path)
            .await
            .map_err(|refusal| refusal.of_move(path, &device.name))
    }

    /// The user's paired device `device_id`.
    async fn paired_device(&self, device_id: &DeviceId) -> Result<DeviceRecord, ChangeRefusal> {
        self.control()
            .device(device_id.clone())
            .await?
            .filter(|device| device.user == *self.user() && device.kind == DeviceKind::User)
            .ok_or(ChangeRefusal::DeviceNotFound)
    }

    /// A move's destination `to` checked with one question to its Host, and
    /// its directory as the Host names it
    /// (`docs/decisions/0027-a-move-applies-at-once.md`):
    ///
    /// - A directory of a paired device, given or a project's, is checked and
    ///   named by the device's runner; a project keeps its own id.
    /// - A Cloud directory a move names, as `demi host move` does, is checked
    ///   and named by the Cloud, which this wakes.
    /// - The Cloud's directory of a conversation and a Cloud project's are ones
    ///   the backend made, and asking would wake the Cloud for nothing.
    pub async fn move_destination(
        &self,
        to: ConversationTarget,
    ) -> Result<ConversationTarget, ChangeRefusal> {
        match to {
            ConversationTarget::Device { device_id, path } => {
                let device = self.paired_device(&device_id).await?;
                let path = self.checked_directory(&device, &path).await?;
                Ok(ConversationTarget::Device { device_id, path })
            }
            ConversationTarget::Workspace { workspace_id } => {
                let workspace = self
                    .control()
                    .workspace(workspace_id.clone())
                    .await?
                    .filter(|workspace| workspace.user == *self.user())
                    .ok_or(ChangeRefusal::WorkspaceNotFound)?;
                let device = self
                    .control()
                    .device(workspace.device.clone())
                    .await?
                    .ok_or(ChangeRefusal::DeviceNotFound)?;
                if device.kind == DeviceKind::User {
                    self.checked_directory(&device, &workspace.path).await?;
                }
                Ok(ConversationTarget::Workspace { workspace_id })
            }
            ConversationTarget::Cloud { path: Some(path) } => {
                let device = self.cloud_shard().cloud_device().await?;
                let path = self.checked_directory(&device, &path).await?;
                Ok(ConversationTarget::Cloud { path: Some(path) })
            }
            ConversationTarget::Cloud { path: None } => Ok(ConversationTarget::Cloud { path: None }),
        }
    }

    /// `path` on `device` as its Host names it, which must be an absolute
    /// directory there.
    async fn checked_directory(&self, device: &DeviceRecord, path: &str) -> Result<String, ChangeRefusal> {
        let host = if device.kind == DeviceKind::Managed { "Cloud" } else { device.name.as_str() };
        if !host_paths(device.platform).is_absolute(path) {
            return Err(ChangeRefusal::NotADirectory(format!("{path}: give an absolute directory on {host}")));
        }
        self.canonical_directory(device, path)
            .await
            .map_err(|refusal| refusal.of_move(path, host))
    }
}

/// The answer to `question`, none when it does not come within
/// [`QUESTION_DEADLINE`].
async fn asked<T>(question: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout(QUESTION_DEADLINE, question).await.ok()
}
