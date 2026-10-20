//! A directory as its Host names it (`crates/backend-database/src/selection.rs`): every move asks the destination's Host, which follows links,
//! drops a trailing separator and spells the directory as its file system
//! keeps it, so two spellings of one directory select the one project, and
//! a home reached through a link is the home.

use demi_backend_database::devices::DeviceRecord;
use demi_host_interface::{FileKind, HostError, HostErrorKind, HostFs};
use demi_runner_protocol::files::FsFailure;
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::DeviceId;

use crate::HostShard;
use crate::transition::ChangeRefusal;

/// Why a Host named no directory for a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryRefusal {
    /// The Host is not reachable now; the text says why.
    Offline(String),
    /// Nothing is there, or it is not a directory.
    NotADirectory,
    /// The Host failed otherwise; the text is its.
    Failed(String),
}

impl DirectoryRefusal {
    /// The refusal of a move to `directory` on the device `host`.
    pub fn of_move(self, directory: &str, host: &str) -> ChangeRefusal {
        match self {
            Self::Offline(reason) => ChangeRefusal::DeviceOffline(reason),
            Self::NotADirectory => ChangeRefusal::NotADirectory(format!("{directory} is not a directory on {host}")),
            Self::Failed(reason) => ChangeRefusal::HostFailed(format!("{directory} on {host}: {reason}")),
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
                resolve(&access.host, directory).await
            }
            DeviceKind::User => {
                let Some(access) = self.devices().device_access(&device.id) else {
                    return Err(DirectoryRefusal::Offline(self.offline(device).message));
                };
                resolve(&access, directory).await
            }
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

    /// `to` with its directory as the Host names it: a directory of a
    /// paired device, which the device's runner resolves; the Cloud's
    /// directory of a conversation, a Fork's source's or a Cloud project's
    /// is one the backend made and names as it is, and `demi host move`
    /// resolves a Cloud directory it is given itself.
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
        let device = self
            .control()
            .device(device_id.clone())
            .await?
            .filter(|device| device.user == *self.user() && device.kind == DeviceKind::User)
            .ok_or(ChangeRefusal::DeviceNotFound)?;
        self.canonical_directory(&device, path)
            .await
            .map_err(|refusal| refusal.of_move(path, &device.name))
    }
}

/// `directory`'s real path on the Host `fs`, when it is a directory.
async fn resolve(fs: &dyn HostFs, directory: &str) -> Result<String, HostError> {
    let real = fs.realpath(directory).await?;
    let stat = fs.stat(&real).await?;
    if stat.kind == FileKind::Directory {
        Ok(real)
    } else {
        Err(HostError::failed(Some("ENOTDIR".into()), format!("{real} is not a directory")))
    }
}
