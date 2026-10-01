//! Where a conversation's work runs (`sessions-and-targets.md` § Resolve a
//! target): its selection resolved to a device and the directory its work
//! starts in there.

use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::{ConversationRecord, ExecutionTarget};
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::ids::ConversationId;

use crate::HostShard;

/// The Cloud's home directory, before its runner reports one.
const CLOUD_HOME: &str = "/home/demi";

/// Where a conversation on the Cloud works unless it names a directory.
pub(crate) fn cloud_session_directory(id: &ConversationId, home: Option<&str>) -> String {
    format!("{}/sessions/{id}", home.unwrap_or(CLOUD_HOME))
}

impl dyn HostShard + '_ {
    /// The conversation's selection as its device and directory. Resolving
    /// never starts a machine.
    pub async fn resolve_target(
        &self,
        record: &ConversationRecord,
    ) -> Result<ExecutionTarget, StorageError> {
        let control = self.control();
        match &record.target {
            ConversationTarget::Workspace { workspace_id } => {
                // A workspace stays while conversations target it.
                let workspace = control
                    .workspace(workspace_id.clone())
                    .await?
                    .filter(|workspace| workspace.user == record.owner)
                    .ok_or_else(|| StorageError::Corrupt {
                        table: "conversations",
                        column: "target_workspace_id",
                        reason: format!("workspace {workspace_id} is not one of the owner's"),
                    })?;
                Ok(ExecutionTarget::Workspace {
                    workspace_id: workspace.id,
                    device_id: workspace.device,
                    path: workspace.path,
                })
            }
            ConversationTarget::Device { device_id, path } => Ok(ExecutionTarget::Device {
                device_id: device_id.clone(),
                path: path.clone(),
            }),
            ConversationTarget::Cloud { path } => {
                let device = control
                    .managed_device(record.owner.clone())
                    .await?
                    .map(|device| device.id);
                let path = match path {
                    Some(path) => path.clone(),
                    None => {
                        let home = device
                            .as_ref()
                            .and_then(|device| self.devices().home(device));
                        cloud_session_directory(&record.id, home.as_deref())
                    }
                };
                Ok(ExecutionTarget::Cloud {
                    device_id: device,
                    path,
                })
            }
        }
    }
}
