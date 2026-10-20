//! How the user's conversations use the Cloud: as the `target` their files and
//! commands are on, or `attached` as an extra Host. Every role keeps the
//! Cloud awake while the conversation works; an idle stop and a reset hold
//! the conversations whose files are on the Cloud and leave an attached one
//! alone: it runs on its own target. A conversation's `provider` role names
//! the Claude Code machine, never the user's Cloud, and the work on that
//! machine holds it instead (`managed-hosts.md` § The Claude Code machine).
//! The `provider`
//! role is derived, never stored: a node of the conversation's tree, the root
//! or any subagent, infers with a provider that needs a process on a Host,
//! and the provider's placement says which device that is; the same placement
//! starts the process, so the role and the process cannot name different
//! machines. The lifecycle knows roles; it does not know providers.
//!
//! A reset reaches a conversation through the `target` and `provider` roles
//! only, since those conversations cannot work without the device. A
//! conversation that merely has the Cloud attached keeps running on its own
//! target: its turn is not interrupted, its streams stay open and it can be
//! opened, read and written throughout. A command it had running on the Cloud
//! ends with the device, as on any Host that went away, and its next Cloud
//! command waits for the reset like any other wake and then runs, never
//! refused because a reset is running.
//!
//! A held conversation is waiting, not failed. While a transition holds it (a
//! reset, a target change, an archive), everything the user sends to it waits
//! for the transition and then proceeds (opening it, a message, a steer, a
//! rename, a pin, a model change, a Host operation): nothing is answered with
//! a refusal because the transition is running, so nothing is left for the
//! user to retry. A wait ends early only when its requester goes away. User
//! streams, the one-shot user calls admitted like them and file transfers are
//! the exception: a transition ends them instead of waiting, and the page
//! reopens its streams by itself afterwards. A request for another
//! transition (a target change, a detach, an archive) does not wait either: it
//! answers busy.

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
