//! A workspace on the user's Cloud: a new project directory there, made through
//! the Cloud's machine access (`sessions-and-targets.md` § Every way to a Host),
//! which wakes a stopped Cloud.

use demi_backend_database::workspaces::{WorkspaceCreation, WorkspaceRecord, new_workspace_id};
use demi_host_interface::{Host as _, HostFs, MkdirOptions};

use crate::shard::Shard;
use demi_backend_host_access::access::{HostAccessError, Refusal};

impl Shard {
    /// Makes `~/projects/<id>` on the user's Cloud, then the workspace `name`
    /// over it. Every project of the user is a directory on the same Cloud.
    pub async fn create_cloud_workspace(
        &self,
        name: String,
    ) -> Result<WorkspaceRecord, HostAccessError> {
        let id = new_workspace_id();
        let access = self.cloud_shard().machine_access().await?;
        let path = access
            .host
            .identity()
            .paths
            .join(&access.home, &format!("projects/{id}"));
        HostFs::mkdir(&access.host, &path, MkdirOptions { recursive: true }).await?;
        match self
            .services()
            .control
            .create_workspace(
                id,
                self.user().clone(),
                access.device.id.clone(),
                path,
                name,
            )
            .await?
        {
            WorkspaceCreation::Created(workspace) => Ok(workspace),
            // The directory is new, so neither its project nor its home.
            WorkspaceCreation::DeviceNotFound | WorkspaceCreation::Exists(_) | WorkspaceCreation::Home => {
                Err(Refusal::DeviceGone.into())
            }
        }
    }
}
