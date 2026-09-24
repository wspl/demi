//! A workspace on the user's Cloud (`web-api.md` § Workspaces, devices, and
//! attached hosts): a new project directory there, made through the Cloud's
//! machine access (`sessions-and-targets.md` § Every way to a Host), which
//! wakes a stopped Cloud.

use demi_shell::{HostFs, MkdirOptions};

use super::host_access::{HostAccessError, Refusal};
use crate::shard::Shard;
use crate::storage::workspaces::{WorkspaceRecord, new_workspace_id};

impl Shard {
    /// Makes `~/projects/<id>` on the user's Cloud, then the workspace `name`
    /// over it. Every project of the user is a directory on the same Cloud.
    pub(crate) async fn create_cloud_workspace(&self, name: String) -> Result<WorkspaceRecord, HostAccessError> {
        let id = new_workspace_id();
        let access = self.machine_access().await?;
        let path = format!("{}/projects/{id}", access.home);
        HostFs::mkdir(&access.host, &path, MkdirOptions { recursive: true }).await?;
        self.services()
            .control
            .create_workspace(id, self.user().clone(), access.device.id.clone(), path, name)
            .await?
            .ok_or_else(|| Refusal::DeviceGone.into())
    }
}
