//! Where a conversation's work runs (`sessions-and-targets.md` § Resolve a
//! target): its selection resolved to a device and the directory its work
//! starts in there.

use demi_web_api::conversations::ConversationTarget;
use demi_web_api::ids::{ConversationId, DeviceId, WorkspaceId};
use serde::{Deserialize, Serialize};

use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::ConversationRecord;

/// The Cloud's home directory, before its runner reports one.
const CLOUD_HOME: &str = "/home/demi";

/// Where a conversation on the Cloud works unless it names a directory.
pub(crate) fn cloud_session_directory(id: &ConversationId, home: Option<&str>) -> String {
    format!("{}/sessions/{id}", home.unwrap_or(CLOUD_HOME))
}

/// A conversation's selection resolved: the device its work runs on, and
/// the directory the work starts in there. A Cloud has no device until its
/// first use makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub(crate) enum ExecutionTarget {
    Cloud {
        device_id: Option<DeviceId>,
        path: String,
    },
    Device {
        device_id: DeviceId,
        path: String,
    },
    Workspace {
        workspace_id: WorkspaceId,
        device_id: DeviceId,
        path: String,
    },
}

impl ExecutionTarget {
    pub(crate) fn device(&self) -> Option<&DeviceId> {
        match self {
            Self::Cloud { device_id, .. } => device_id.as_ref(),
            Self::Device { device_id, .. } | Self::Workspace { device_id, .. } => Some(device_id),
        }
    }

    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Cloud { path, .. } | Self::Device { path, .. } | Self::Workspace { path, .. } => path,
        }
    }
}

/// The latest target switch, which every node's next context block
/// describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TargetSwitch {
    pub(crate) from: ExecutionTarget,
    pub(crate) to: ExecutionTarget,
}

impl Shard {
    /// The conversation's selection as its device and directory. Resolving
    /// never starts a machine.
    pub(crate) async fn resolve_target(&self, record: &ConversationRecord) -> Result<ExecutionTarget, StorageError> {
        let control = &self.services().control;
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
                let device = control.managed_device(record.owner.clone()).await?.map(|device| device.id);
                let path = match path {
                    Some(path) => path.clone(),
                    None => {
                        let home = device.as_ref().and_then(|device| self.devices().home(device));
                        cloud_session_directory(&record.id, home.as_deref())
                    }
                };
                Ok(ExecutionTarget::Cloud { device_id: device, path })
            }
        }
    }
}
