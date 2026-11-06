//! The core's side of conversation permissions.
//!
//! - What the check, the decisions and their messages need of the core
//!   (`PermissionCore`).
//! - The delivery at start of the decisions a restart cut off.

use std::rc::Rc;

use demi_core_store::store::CoreStore;
use demi_core_store::permissions::AskingAgent;
use demi_core_host_access::root_of;
use demi_core_sync::UserMarks;
use demi_core_sync::Revisions;
use demi_core_permissions::{PermissionCore, Waits};
use demi_command_declarations::{Category, Node};
use demi_shared_types::{AgentMessage, NodeId};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::permissions::{PermissionRequestId, RequestingAgent};
use demi_web_api_protocol::push::PushAsk;
use futures_util::future::LocalBoxFuture;

use super::Core;

impl PermissionCore for Core {
    fn store(&self) -> &CoreStore {
        Core::store(self)
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

    fn this(&self) -> Rc<dyn PermissionCore> {
        Core::this(self)
    }

    fn waits(&self) -> &Waits {
        &self.waits
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

impl Core {
    /// The core as the conversation permissions see it, whose operations
    /// it is.
    pub fn permission_core(&self) -> &(dyn PermissionCore + 'static) {
        self
    }
}

