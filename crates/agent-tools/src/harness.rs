//! What a product supplies for its agents (`runtime.md` § Sessions and
//! turns): the harness names a node's system prompt, the text added to each
//! of its user turns, the context text that announces a change of the
//! conversation's execution context, and the commands its shell offers.
//! Demi has one harness, the coding agent.

use std::rc::Rc;

use demi_core::{ModelSelection, NodeId};
use demi_shell::{CommandSet, Host, HostError, HostErrorKind};

/// The node a harness hook is asked about.
#[derive(Debug, Clone, Copy)]
pub struct PromptContext<'a> {
    /// The node whose request or turn it is.
    pub node: &'a NodeId,
    /// The conversation's root node. Every node of a tree runs on the
    /// conversation's execution target, which belongs to the root.
    pub root: &'a NodeId,
    /// The node's working directory.
    pub cwd: &'a str,
}

/// A product's agent. Its hooks run on the user's shard, so their futures
/// need not be `Send`, and the agent calls them through the concrete type.
#[expect(
    async_fn_in_trait,
    reason = "a harness runs on the user's shard: its futures are never sent to another thread"
)]
pub trait AgentHarness: 'static {
    /// The Hosts the conversations' shell tools run on. The agent only asks
    /// one for its key; the product's shell environment factory
    /// (`ServerDeps::shells`) makes a node's environment on it.
    type Host: Host;

    /// The harness's name, which every checkpoint records: a node another
    /// harness saved is not restored.
    fn name(&self) -> &str;

    /// The Host a node's shell tools reach now: the conversation's current
    /// execution target (`sessions-and-targets.md` § Host operations). Two
    /// answers for the same target have equal keys, so a node keeps one
    /// shell environment per target it used. By default the harness has
    /// none, and a shell tool's call fails.
    async fn host(&self, _context: PromptContext<'_>) -> Result<Rc<Self::Host>, HostError> {
        Err(HostError::new(
            HostErrorKind::Unavailable,
            "this agent runs no shell tools",
        ))
    }

    /// The commands a node's shell offers. A node renders their help into
    /// its system prompt once, when it is assembled.
    fn commands(&self) -> Rc<CommandSet>;

    /// The named subagent profiles `demi agent spawn --profile` selects
    /// (`subagents.md` § Profiles); none by default. Omitting `--profile`
    /// always inherits the parent, and the name `default` is reserved.
    fn profiles(&self) -> Vec<Profile> {
        Vec::new()
    }

    /// The system prompt of a node's requests. `commands` is the rendered
    /// help of the node's commands, empty when it has none.
    async fn system_prompt(&self, context: PromptContext<'_>, commands: &str) -> String;

    /// The text the model receives before the content of each user turn;
    /// none by default.
    async fn preamble(&self, _context: PromptContext<'_>) -> Option<String> {
        None
    }

    /// Before each request: the text that tells the model the
    /// conversation's execution context changed since the node last saw it,
    /// such as a target switch; none when nothing changed, and by default.
    /// `seen` is the text of each context block the node's transcript holds,
    /// oldest first: what the node saw.
    async fn context(&self, _context: PromptContext<'_>, _seen: &[&str]) -> Option<String> {
        None
    }
}

/// A named subagent configuration (`subagents.md` § Profiles): every field
/// overrides what the child would inherit from its parent.
#[derive(Clone)]
pub struct Profile {
    /// The `--profile` value.
    pub name: String,
    /// What the profile is for, listed in the spawn command's help.
    pub description: String,
    /// Replaces the parent's system prompt and drops its preamble; it is
    /// given the node's context and the rendered help of its commands.
    pub system_prompt: Option<Rc<dyn Fn(PromptContext<'_>, &str) -> String>>,
    /// Narrows the parent's harness commands for the child.
    pub commands: Option<Rc<dyn Fn(&CommandSet) -> CommandSet>>,
    /// Whether the profile's children may spawn children of their own.
    pub can_spawn_subagents: bool,
    /// A model used instead of the parent's, on a fork of the parent's
    /// provider runtime.
    pub model: Option<ModelSelection>,
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Profile")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("can_spawn_subagents", &self.can_spawn_subagents)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}
