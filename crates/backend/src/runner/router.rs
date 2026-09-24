//! Each agent node's commands, for the rpc calls its jobs make
//! (`commands.md` § Handle an rpc call). A node registers its command set
//! and command storage while one of its shell environments lives; a job's
//! call names its node, and the relay dispatches the call to that node's
//! commands, provided the node belongs to the job's conversation. The
//! registration is removed with the last environment that holds it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_host_remote::JobOrigin;
use demi_shell::{CommandSet, PortError, RpcError, RpcInvocation, RpcPort, StorageOp, StorageReply};
use demi_web_api::ids::ConversationId;
use futures_util::future::LocalBoxFuture;

/// An agent node's command storage (`command-state-history.md`), which the
/// agent keeps; a job's calls read and write it at the history generation
/// the job started in.
pub(crate) trait NodeStorage {
    fn apply(&self, generation: u64, op: StorageOp) -> LocalBoxFuture<'_, Result<StorageReply, PortError>>;
}

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
    storage: Rc<dyn NodeStorage>,
    nodes: Weak<RefCell<HashMap<String, Weak<Node>>>>,
}

/// A node's hold on its registration; the last one dropped removes it.
#[derive(Clone)]
#[expect(dead_code, reason = "a node's shell environments hold its registration")]
pub(crate) struct CommandRegistration(Rc<Node>);

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
    /// Registers `node` of `conversation` with its commands and storage. A
    /// node registered already keeps its registration, which the answer
    /// shares.
    #[expect(dead_code, reason = "a node's shell environments register its commands")]
    pub(crate) fn register(
        &self,
        node: &str,
        conversation: &ConversationId,
        commands: Rc<CommandSet>,
        storage: Rc<dyn NodeStorage>,
    ) -> CommandRegistration {
        let mut nodes = self.nodes.borrow_mut();
        if let Some(registered) = nodes.get(node).and_then(Weak::upgrade) {
            return CommandRegistration(registered);
        }
        let registered = Rc::new(Node {
            id: node.to_owned(),
            conversation: conversation.clone(),
            commands,
            storage,
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

    /// One operation of `job` on its node's command storage.
    pub(crate) fn storage(&self, job: &JobOrigin, op: StorageOp) -> LocalBoxFuture<'static, Result<StorageReply, PortError>> {
        let node = self.node_of(job);
        let generation = job.caller.as_ref().map(|caller| caller.generation);
        Box::pin(async move {
            let node = node.map_err(PortError::Storage)?;
            let generation = generation.ok_or_else(|| PortError::Storage("the job has no command storage".into()))?;
            node.storage.apply(generation, op).await
        })
    }
}
