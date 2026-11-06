//! The shard's side of conversation permissions.
//!
//! - What the check, the decisions and their messages need of the shard
//!   (`PermissionShard`).
//! - The delivery at start of the decisions a restart cut off.

use demi_web_api_protocol::ids::UserId;
use std::rc::Rc;
use std::sync::Arc;

use demi_core_store::StorageError;
use demi_core_store::store::CoreStore;
use demi_core_store::credentials::CredentialSeal;
use demi_core_store::permissions::AskingAgent;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::UserMarks;
use demi_backend_page_sync::Revisions;
use demi_backend_permissions::{PermissionShard, Waits, deliver};
use demi_command_declarations::{Category, Node};
use demi_shared_types::{AgentMessage, NodeId};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::permissions::{PermissionRequestId, RequestingAgent};
use demi_web_api_protocol::push::PushAsk;
use futures_util::future::LocalBoxFuture;

use super::{Shard, Shards};

impl PermissionShard for Shard {
    fn store(&self) -> &CoreStore {
        Shard::store(self)
    }

    fn marks(&self) -> UserMarks {
        self.services().sync.of(self.user())
    }

    fn revisions(&self) -> &Revisions {
        &self.permission_revisions
    }

    fn categories(&self) -> LocalBoxFuture<'_, Result<Vec<Category>, String>> {
        Box::pin(async move {
            // The agent runtime's groups declare categories of their own.
            let commands = self.agent().node_commands().await?;
            Ok(commands
                .declarations()
                .flat_map(Node::categories)
                .cloned()
                .collect())
        })
    }

    fn asking_agent(
        &self,
        conversation: &ConversationId,
        node: &NodeId,
    ) -> Result<AskingAgent, String> {
        let live = self
            .agent()
            .node(&root_of(conversation), node)
            .ok_or_else(|| format!("no agent session behind node {node}"))?;
        let record = live.record();
        let subagent = record.parent.is_some().then(|| RequestingAgent {
            number: record.number,
            description: record.description.clone(),
        });
        Ok(AskingAgent {
            node: node.clone(),
            subagent,
        })
    }

    fn admit<'a>(
        &'a self,
        conversation: &'a ConversationId,
        asking: &'a NodeId,
        message: &'a dyn Fn(&NodeId) -> AgentMessage,
    ) -> LocalBoxFuture<'a, Result<NodeId, String>> {
        Box::pin(async move {
            let root = root_of(conversation);
            if self.agent().tree(&root).is_none() {
                // Counted before the open, so the close waits for it before
                // the agent shuts down.
                let _opening = self.tree_openers().token();
                if self.is_closing() {
                    return Err("the backend is shutting down".into());
                }
                self.restore_tree(conversation).await?;
            }
            self.agent().admit_from_user(&root, asking, message).await
        })
    }

    fn spawn(&self, task: LocalBoxFuture<'static, ()>) {
        self.tasks().spawn_local(task);
    }

    fn this(&self) -> Rc<dyn PermissionShard> {
        Shard::this(self)
    }

    fn waits(&self) -> &Waits {
        &self.waits
    }

    fn credential_seal(&self) -> Arc<dyn CredentialSeal> {
        self.vault_seal()
    }

    fn credentials_changed(&self) {
        self.mark_credentials();
    }

    fn asked(&self, conversation: &ConversationId, request: PermissionRequestId, ask: PushAsk) {
        self.notify_asked(conversation, request, ask);
    }

    fn raised(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
        })
    }
}

impl Shard {
    /// The shard as the conversation permissions see it, whose operations
    /// it is.
    pub fn permission_shard(&self) -> &(dyn PermissionShard + 'static) {
        self
    }
}

/// Delivers at start each decision whose message a restart cut off: each
/// user's shard sends it to the agent that asked, opening its conversation's
/// tree. Answers once each shard has its tasks.
pub async fn deliver_decisions(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        for request in store.undelivered_permissions().await? {
            let handed = shards
                .of(owner)
                .adopt(move |shard| {
                    let shard: Rc<dyn PermissionShard> = shard;
                    deliver(shard, request)
                })
                .await;
            if handed.is_err() {
                // The backend is shutting down; the next start delivers what
                // is left.
                return Ok(());
            }
        }
    }
    Ok(())
}
