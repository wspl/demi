//! The shard's side of conversation permissions (`permissions.md`
//! § Responsibilities): what the check, the decisions and their messages
//! need of the shard (`PermissionShard`), and the delivery at start of the
//! decisions a restart cut off.

use std::rc::Rc;
use std::sync::Arc;

use demi_agent_tools::ToolsetSource;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::credentials::CredentialSeal;
use demi_backend_database::permissions::{AskingAgent, Undelivered};
use demi_backend_host_access::root_of;
use demi_backend_page_sync::UserMarks;
use demi_backend_permissions::{PermissionShard, Revisions, Waits, deliver};
use demi_command_declarations::{Category, Node};
use demi_shared_types::{AgentMessage, NodeId};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::permissions::RequestingAgent;
use futures_util::future::LocalBoxFuture;

use super::{Shard, Shards};
use crate::conversation::product::ShardToolsets;

impl PermissionShard for Shard {
    fn control(&self) -> &ControlService {
        &self.services().control
    }

    fn marks(&self) -> UserMarks {
        self.services().sync.of(self.user())
    }

    fn revisions(&self) -> &Revisions {
        &self.permission_revisions
    }

    fn categories(&self) -> LocalBoxFuture<'_, Result<Vec<Category>, String>> {
        Box::pin(async move {
            let toolsets = ShardToolsets {
                shard: self.this.clone(),
            };
            let toolset = toolsets.current().await?;
            // The agent runtime's groups declare categories of their own.
            let commands = self
                .agent()
                .node_commands(&toolset.commands)
                .map_err(|error| error.to_string())?;
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
        &self.credential_waits
    }

    fn credential_seal(&self) -> Arc<dyn CredentialSeal> {
        self.vault_seal()
    }

    fn credentials_changed(&self) {
        self.mark_credentials();
    }
}

impl Shard {
    /// The shard as the conversation permissions see it, whose operations
    /// it is.
    pub fn permission_shard(&self) -> &(dyn PermissionShard + 'static) {
        self
    }

    /// Notes that the conversation `id`'s Workflow menu choice changed.
    pub(crate) fn workflow_choice_changed(&self, id: &ConversationId) {
        *self.workflow_choices.borrow_mut().entry(id.clone()).or_default() += 1;
    }

    /// How often the conversation `id`'s choice changed in this run.
    pub(crate) fn workflow_choice_changes(&self, id: &ConversationId) -> u64 {
        self.workflow_choices.borrow().get(id).copied().unwrap_or(0)
    }

    pub(crate) fn permission_revisions(&self) -> &Revisions {
        &self.permission_revisions
    }
}

/// Delivers at start each decision whose message a restart cut off
/// (`permissions.md` § The decision's message): each owner's shard sends it
/// to the agent that asked, opening its conversation's tree. Answers once
/// each shard has its tasks.
pub async fn deliver_decisions(control: &ControlService, shards: &Shards) -> Result<(), StorageError> {
    for Undelivered { owner, request } in control.undelivered_permissions().await? {
        let handed = shards
            .of(&owner)
            .adopt(move |shard| {
                let shard: Rc<dyn PermissionShard> = shard;
                deliver(shard, request)
            })
            .await;
        if handed.is_err() {
            // The backend is shutting down; the next start delivers what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}
