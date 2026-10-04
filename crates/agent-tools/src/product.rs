//! What a product supplies for its agents (`runtime.md` § Sessions and
//! turns) besides the data the agent server is given: the commands a tree
//! opens with, the user's subagent settings each spawn reads, the Host a
//! node's shell tools reach, and the context sources asked before each
//! request.

use std::rc::Rc;

use demi_host_interface::{CommandSet, Host, HostError, HostErrorKind};
use demi_shared_types::{NodeId, TurnId};
use futures_util::future::LocalBoxFuture;

/// The commands every node of a tree starts from, which a tree takes when
/// it opens and keeps until it closes; `revision` tells two toolsets with
/// other commands apart.
#[derive(Clone)]
pub struct Toolset {
    pub commands: Rc<CommandSet>,
    pub revision: Rc<str>,
}

/// Where a tree that opens takes its toolset from: the product's current
/// one, which may change while the server runs.
pub trait ToolsetSource {
    fn current(&self) -> LocalBoxFuture<'_, Result<Toolset, String>>;
}

/// A toolset that never changes.
impl ToolsetSource for Toolset {
    fn current(&self) -> LocalBoxFuture<'_, Result<Toolset, String>> {
        let toolset = self.clone();
        Box::pin(async move { Ok(toolset) })
    }
}

/// The user's subagent settings at one moment (`subagents.md` § Profiles):
/// the Subagent switch and the user's profiles, in name order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSettings {
    /// Whether the user has subagents on.
    pub enabled: bool,
    pub profiles: Vec<Profile>,
}

/// One of the user's subagent profiles, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The `--profile` value.
    pub name: String,
    /// When the agent should use it, which `demi agent profiles` shows.
    pub description: String,
    /// The model settings a child infers with; none for its parent's model.
    pub model: Option<ProfileModel>,
    /// The text that replaces the instructions in a child's system prompt;
    /// none for its parent's.
    pub instructions: Option<String>,
    /// Whether its children may spawn children of their own.
    pub can_spawn: bool,
    /// Whether agents may use it now.
    pub enabled: bool,
}

/// A profile's model settings (`models.md` § A conversation's model
/// settings): the provider entry, the model, the thinking effort and the
/// service tier the user chose, which the product builds a child's model
/// selection from at each spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileModel {
    pub provider_id: String,
    pub model_id: String,
    /// An effort the model listed, or thinking off; none only for a model
    /// that lists no efforts.
    pub thinking_effort: Option<String>,
    /// A tier the model listed; none for the vendor's default.
    pub service_tier_id: Option<String>,
}

/// What of a profile's model settings is missing now, which makes the
/// profile unavailable (`subagents.md` § An unavailable profile). Each says
/// what the user changes in settings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    #[error(
        "its provider entry is gone or no longer available to the user; choose another model for the profile in settings"
    )]
    Entry,
    #[error(
        "model \"{0}\" is no longer in its provider entry's catalog; choose another model for the profile in settings"
    )]
    Model(String),
    #[error(
        "model \"{model}\" no longer offers the effort \"{effort}\"; choose another effort for the profile in settings"
    )]
    Effort { model: String, effort: String },
    #[error(
        "model \"{model}\" no longer offers the service tier \"{tier}\"; choose another service tier for the profile in settings"
    )]
    Tier { model: String, tier: String },
    /// The settings could not be checked, such as a storage read that
    /// failed; the spawn fails with it as with a missing part.
    #[error("its model settings could not be checked: {0}")]
    Failed(String),
}

/// Where the agent reads the user's subagent settings: at each spawn and
/// each `demi agent profiles`, never once per tree.
pub trait SubagentSource {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>>;
}

/// Settings that never change.
impl SubagentSource for SubagentSettings {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>> {
        let settings = self.clone();
        Box::pin(async move { Ok(settings) })
    }
}

/// The node a question is about.
#[derive(Debug, Clone, Copy)]
pub struct NodeContext<'a> {
    /// The node whose request or tool call it is.
    pub node: &'a NodeId,
    /// The conversation's root node. Every node of a tree runs on the
    /// conversation's execution target, which belongs to the root.
    pub root: &'a NodeId,
    /// The node's working directory.
    pub cwd: &'a str,
}

/// Where a conversation's nodes run. Its futures run on the user's shard, so
/// they need not be `Send`, and the agent calls it through the concrete type.
#[expect(
    async_fn_in_trait,
    reason = "a resolver runs on the user's shard: its futures are never sent to another thread"
)]
pub trait HostResolver: 'static {
    /// The Hosts the conversations' shell tools run on. The agent only asks
    /// one for its key; the product's shell environment factory
    /// (`ServerDeps::shells`) makes a node's environment on it.
    type Host: Host;

    /// The Host a node's shell tools reach now: the conversation's current
    /// execution target (`sessions-and-targets.md` § Host operations). Two
    /// answers for the same target have equal keys, so a node keeps one
    /// shell environment per target it used. By default there is none, and a
    /// shell tool's call fails.
    async fn host(&self, _context: NodeContext<'_>) -> Result<Rc<Self::Host>, HostError> {
        Err(HostError::new(
            HostErrorKind::Unavailable,
            "this agent runs no shell tools",
        ))
    }
}

/// One source of what the model must learn before a request (`runtime.md`
/// § Context), such as the conversation's execution context or a plugin.
pub trait ContextSource {
    /// The name its blocks record: `execution`, or a plugin's id.
    fn name(&self) -> &str;

    /// What the node must learn now, or nothing. `seen` is the text of the
    /// source's own blocks the model receives, oldest first, and `turn` the
    /// input turn the request belongs to. A failure adds no block; the
    /// source is asked again before the next request.
    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<String>, String>>;
}
