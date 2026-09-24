//! A workspace on the user's Cloud (`web-api.md` § Workspaces, devices, and
//! attached hosts): a new project directory there, made through the Cloud's
//! machine access (`sessions-and-targets.md` § Every way to a Host), which
//! can wake it.

use super::host_access::{CloudUnavailable, HostAccessError};
use crate::shard::Shard;
use crate::storage::workspaces::WorkspaceRecord;

impl Shard {
    /// Makes a project directory on the user's Cloud and the workspace
    /// `name` over it. The Cloud's machine access comes with its lifecycle;
    /// until then no Cloud starts, and the creation answers so.
    pub(crate) async fn create_cloud_workspace(&self, name: String) -> Result<WorkspaceRecord, HostAccessError> {
        Err(CloudUnavailable(format!("The Cloud cannot start to make the workspace {name:?}")).into())
    }
}
