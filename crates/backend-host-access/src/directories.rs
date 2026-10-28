//! A directory as the user types it, which a move, a new conversation and a
//! new project store at once, and as its Host first names it
//! (`crates/backend-host-access/src/access.rs`).
//!
//! A destination is stored as typed, with `~` expanded to the Host's home and a
//! trailing separator dropped, and the move or the creation answers without
//! asking a Host: a Host that is slow, offline or a stopped Cloud holds nothing.
//! The first use of the directory on a connection asks its runner to create
//! it with its parents and name it as its file system keeps it, links followed;
//! the name then replaces what was typed, and the project of a directory that
//! another project has is that project.

use demi_backend_database::devices::DeviceRecord;
use demi_runner_client::host_paths;
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::DeviceId;

use crate::HostShard;
use crate::transition::ChangeRefusal;

impl dyn HostShard + '_ {
    /// `to` with its directory as typed, as a new conversation starts there.
    /// The Cloud's directory of a conversation, a Fork's source's or a Cloud
    /// project's is one the backend made and names as it is, and a project's
    /// is its own.
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

    /// `path` on the user's paired device `device_id` as typed.
    pub async fn device_directory(&self, device_id: &DeviceId, path: &str) -> Result<String, ChangeRefusal> {
        let device = self.paired_device(device_id).await?;
        typed_directory(&device, path)
    }

    /// The user's paired device `device_id`.
    async fn paired_device(&self, device_id: &DeviceId) -> Result<DeviceRecord, ChangeRefusal> {
        self.control()
            .device(device_id.clone())
            .await?
            .filter(|device| device.user == *self.user() && device.kind == DeviceKind::User)
            .ok_or(ChangeRefusal::DeviceNotFound)
    }

    /// A move's destination `to` as typed, for the move to store and answer:
    ///
    /// - A directory of a paired device, given or a project's, is stored as it
    ///   is; a project keeps its own id.
    /// - A Cloud directory a move names, as `demi host move` does, is stored
    ///   as typed, and the Cloud is not woken for it.
    /// - The Cloud's directory of a conversation and a Cloud project's are ones
    ///   the backend made.
    pub async fn move_destination(
        &self,
        to: ConversationTarget,
    ) -> Result<ConversationTarget, ChangeRefusal> {
        match to {
            ConversationTarget::Device { device_id, path } => {
                let device = self.paired_device(&device_id).await?;
                let path = typed_directory(&device, &path)?;
                Ok(ConversationTarget::Device { device_id, path })
            }
            ConversationTarget::Workspace { workspace_id } => {
                self.control()
                    .workspace(workspace_id.clone())
                    .await?
                    .filter(|workspace| workspace.user == *self.user())
                    .ok_or(ChangeRefusal::WorkspaceNotFound)?;
                Ok(ConversationTarget::Workspace { workspace_id })
            }
            ConversationTarget::Cloud { path: Some(path) } => {
                let device = self.cloud_shard().cloud_device().await?;
                let path = typed_directory(&device, &path)?;
                Ok(ConversationTarget::Cloud { path: Some(path) })
            }
            ConversationTarget::Cloud { path: None } => Ok(ConversationTarget::Cloud { path: None }),
        }
    }
}

/// `path` as typed on `device`: `~` and `~/…` are the Host's home, and the
/// result must be absolute there.
fn typed_directory(device: &DeviceRecord, path: &str) -> Result<String, ChangeRefusal> {
    let host = if device.kind == DeviceKind::Managed { "Cloud" } else { device.name.as_str() };
    let paths = host_paths(device.platform);
    let path = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => {
            let home = match device.kind {
                DeviceKind::Managed => device.home.as_deref().unwrap_or(demi_backend_database::CLOUD_HOME),
                DeviceKind::User => device.home.as_deref().ok_or(ChangeRefusal::DeviceHomeUnknown)?,
            };
            match rest.trim_start_matches(['/', '\\']) {
                "" => home.to_owned(),
                rest => paths.join(home, rest),
            }
        }
        _ => path.to_owned(),
    };
    if !paths.is_absolute(&path) {
        return Err(ChangeRefusal::NotADirectory(format!("{path}: give an absolute directory on {host}")));
    }
    Ok(path)
}
