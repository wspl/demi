//! How the user's conversations use the Cloud (`sessions-and-targets.md`
//! § How a conversation uses a device): as the `target` their files and
//! commands are on, or `attached` as an extra Host. Every role keeps the
//! Cloud awake while the conversation works; an idle stop and a reset hold
//! the conversations whose files are on the Cloud and leave an attached one
//! alone: it runs on its own target. A conversation's `provider` role names
//! the Claude Code machine, never the user's Cloud, and the work on that
//! machine holds it instead (`managed-hosts.md` § The Claude Code machine).

use std::time::Duration;

use demi_backend_database::StorageError;
use demi_web_api_protocol::ids::ConversationId;

use crate::machine::CloudError;
use crate::{CloudShard, ConversationHold};

/// How a conversation uses the Cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Target,
    Attached,
}

impl dyn CloudShard {
    /// Each of the user's conversations that uses the Cloud, with its
    /// strongest role.
    pub(crate) async fn cloud_uses(&self) -> Result<Vec<(ConversationId, Role)>, StorageError> {
        let control = self.control();
        let cloud = control
            .managed_device(self.user().clone())
            .await?
            .map(|device| device.id);
        let conversations = control.cloud_uses(self.user().clone(), cloud).await?;
        Ok(conversations
            .into_iter()
            .filter_map(|conversation| {
                let role = if conversation.on_cloud {
                    Role::Target
                } else if conversation.attached {
                    Role::Attached
                } else {
                    return None;
                };
                Some((conversation.id, role))
            })
            .collect())
    }

    /// Holds the conversations an idle stop reaches, if none of them works:
    /// each one's tree and file gate reserved now, or nothing held at all.
    pub(crate) fn hold_uses_for_idle(
        &self,
        uses: &[(ConversationId, Role)],
    ) -> Option<Vec<Box<dyn ConversationHold>>> {
        let mut held = Vec::new();
        for (id, role) in uses {
            if *role == Role::Target {
                held.push(self.hold_for_idle(id)?);
            }
        }
        Some(held)
    }

    /// Holds the conversations a reset reaches: each one's turn is
    /// interrupted and its tree held, the file transfers and user streams
    /// of one whose files are on the Cloud end, and its file gate is
    /// reserved once the operations holding it ended. Each wait has `hold`;
    /// a conversation that does not let go within it fails the reset.
    pub(crate) async fn hold_uses_for_reset(
        &self,
        uses: &[(ConversationId, Role)],
        hold: Duration,
    ) -> Result<Vec<Box<dyn ConversationHold>>, CloudError> {
        let mut held = Vec::new();
        for (id, role) in uses {
            if *role != Role::Target {
                continue;
            }
            let conversation = self
                .hold_for_reset(id, true, hold)
                .await
                .ok_or_else(|| {
                    CloudError::Failed(format!("The conversation {id} did not stop for the reset"))
                })?;
            held.push(conversation);
        }
        Ok(held)
    }
}
