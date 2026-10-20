//! Each agent node's commands, for the rpc calls its jobs make
//! (`crates/host-interface/src/rpc.rs`).
//!
//! A node registers its command set while one of its shell environments lives,
//! and replaces it when its commands change, as a plugin turned on or off
//! changes them. A job's call names its node, and the relay dispatches the call
//! to that node's commands now, provided the node belongs to the job's
//! conversation. The registration is removed with the last environment that
//! holds it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_backend_remote_host::{CommandSelection, JobOrigin};
use demi_host_interface::CommandSet;
use demi_web_api_protocol::ids::ConversationId;

/// The registered nodes of one user's shard.
#[derive(Default)]
pub struct CommandRouter {
    nodes: Rc<RefCell<HashMap<String, Weak<Node>>>>,
}

/// One node's commands, while a registration holds them.
struct Node {
    id: String,
    conversation: ConversationId,
    commands: RefCell<Commands>,
    nodes: Weak<RefCell<HashMap<String, Weak<Node>>>>,
}

/// A node's commands, and their manifest, which a job the node starts
/// elsewhere runs with.
struct Commands {
    set: Rc<CommandSet>,
    selection: CommandSelection,
}

/// A node's hold on its registration; the last one dropped removes it.
#[derive(Clone)]
pub struct CommandRegistration(Rc<Node>);

impl CommandRegistration {
    /// The node's commands are `commands`, whose manifest is `selection`,
    /// from now on.
    pub fn replace(&self, commands: Rc<CommandSet>, selection: CommandSelection) {
        self.0.commands.replace(Commands { set: commands, selection });
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let Some(nodes) = self.nodes.upgrade() else {
            return;
        };
        let mut nodes = nodes.borrow_mut();
        // A later registration of the node replaced this one's entry, and
        // stays.
        if nodes
            .get(&self.id)
            .is_some_and(|entry| entry.strong_count() == 0)
        {
            nodes.remove(&self.id);
        }
    }
}

impl CommandRouter {
    /// Registers `node` of `conversation` with its commands and their
    /// manifest. A node registered already keeps its registration, which
    /// the answer shares.
    pub fn register(
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
            commands: RefCell::new(Commands { set: commands, selection }),
            nodes: Rc::downgrade(&self.nodes),
        });
        nodes.insert(node.to_owned(), Rc::downgrade(&registered));
        CommandRegistration(registered)
    }

    /// The registration of the agent node that started `job`, provided the
    /// node belongs to the job's conversation.
    fn node_of(&self, job: &JobOrigin) -> Result<Rc<Node>, String> {
        let node = job
            .caller
            .as_ref()
            .map(|caller| caller.node.as_str())
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
    pub fn selection_of(
        &self,
        node: &str,
        conversation: &ConversationId,
    ) -> Option<CommandSelection> {
        let registered = self.nodes.borrow().get(node).and_then(Weak::upgrade)?;
        (registered.conversation == *conversation).then(|| registered.commands.borrow().selection.clone())
    }

    /// The commands of the agent node that started `job`, in which its
    /// calls run; the backend's dispatch checks a call's permission between
    /// the command set's check and the handler.
    pub fn commands_of(&self, job: &JobOrigin) -> Result<Rc<CommandSet>, String> {
        Ok(self.node_of(job)?.commands.borrow().set.clone())
    }
}
