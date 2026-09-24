//! Each agent node's commands, for the rpc calls its jobs make
//! (`commands.md` § Handle an rpc call). A node registers its command set
//! while one of its shell environments lives; a job's call names its node,
//! and the relay dispatches the call to that node's commands, provided the
//! node belongs to the job's conversation. The registration is removed with
//! the last environment that holds it. A job's command storage is its node's
//! in the agent, which the connection's policy reaches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_host_remote::{CommandSelection, JobOrigin};
use demi_shell::{CommandSet, RpcError, RpcInvocation, RpcPort};
use demi_web_api::ids::ConversationId;
use futures_util::future::LocalBoxFuture;

/// The registered nodes of one user's shard.
#[derive(Default)]
pub(crate) struct CommandRouter {
    nodes: Rc<RefCell<HashMap<String, Weak<Node>>>>,
}

/// One node's commands, while a registration holds them.
struct Node {
    id: String,
    conversation: ConversationId,
    commands: Rc<CommandSet>,
    /// The manifest of `commands`, which a job the node starts elsewhere
    /// runs with.
    selection: CommandSelection,
    nodes: Weak<RefCell<HashMap<String, Weak<Node>>>>,
}

/// A node's hold on its registration; the last one dropped removes it.
#[derive(Clone)]
pub(crate) struct CommandRegistration(#[expect(dead_code, reason = "held for its drop")] Rc<Node>);

impl Drop for Node {
    fn drop(&mut self) {
        let Some(nodes) = self.nodes.upgrade() else {
            return;
        };
        let mut nodes = nodes.borrow_mut();
        // A later registration of the node replaced this one's entry, and
        // stays.
        if nodes.get(&self.id).is_some_and(|entry| entry.strong_count() == 0) {
            nodes.remove(&self.id);
        }
    }
}

impl CommandRouter {
    /// Registers `node` of `conversation` with its commands and their
    /// manifest. A node registered already keeps its registration, which
    /// the answer shares.
    pub(crate) fn register(
        &self,
        node: &str,
        conversation: &ConversationId,
        commands: Rc<CommandSet>,
        selection: CommandSelection,
    ) -> CommandRegistration {
        let mut nodes = self.nodes.borrow_mut();
        if let Some(registered) = nodes.get(node).and_then(Weak::upgrade) {
            return CommandRegistration(registered);
        }
        let registered = Rc::new(Node {
            id: node.to_owned(),
            conversation: conversation.clone(),
            commands,
            selection,
            nodes: Rc::downgrade(&self.nodes),
        });
        nodes.insert(node.to_owned(), Rc::downgrade(&registered));
        CommandRegistration(registered)
    }

    /// The registration of the agent node that started `job`, provided the
    /// node belongs to the job's conversation.
    fn node_of(&self, job: &JobOrigin) -> Result<Rc<Node>, String> {
        let node = job
            .context
            .caller
            .node()
            .ok_or_else(|| "rpc commands run for an agent's jobs".to_owned())?;
        let registered = self
            .nodes
            .borrow()
            .get(node)
            .and_then(Weak::upgrade)
            .ok_or_else(|| format!("no agent session behind node {node}"))?;
        if registered.conversation.as_str() != job.context.conversation {
            return Err(format!("node {node} belongs to another conversation"));
        }
        Ok(registered)
    }

    /// The manifest of `node`'s commands, provided it is registered for
    /// `conversation`: what a job the node starts on another Host runs with.
    pub(crate) fn selection_of(&self, node: &str, conversation: &ConversationId) -> Option<CommandSelection> {
        let registered = self.nodes.borrow().get(node).and_then(Weak::upgrade)?;
        (registered.conversation == *conversation).then(|| registered.selection.clone())
    }

    /// Runs the call `job` made, in its node's commands.
    pub(crate) fn dispatch(
        &self,
        job: &JobOrigin,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>> {
        let node = self.node_of(job);
        Box::pin(async move {
            let node = node.map_err(RpcError::Failed)?;
            node.commands.dispatch(invocation, port).await
        })
    }
}
