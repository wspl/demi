//! What a product supplies for its agents (`runtime.md` § Sessions and
//! turns) besides the data the agent server is given: the commands a tree
//! opens with, the user's subagent settings each spawn reads, the Host a
//! node's shell tools reach, and the context sources asked before each
//! request.

use std::num::NonZeroU32;
use std::rc::Rc;

use demi_conversation_socket_protocol::JobModel;
use demi_host_interface::{CommandSet, Host, HostError, HostErrorKind};
use demi_shared_types::{
    InstructionEntry, ModelSelection, NodeId, ProviderModel, THINKING_OFF, TurnId,
};
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

/// The user's subagent settings at one moment (`subagents.md` § Profiles,
/// § Limits): the Subagents switch, the limits and the user's profiles, in
/// name order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSettings {
    /// Whether the user has subagents on.
    pub enabled: bool,
    /// How many subagents of a conversation may be live at once, at every
    /// depth together; none for Unlimited.
    pub max_live: Option<NonZeroU32>,
    /// How many levels of subagents a tree may have below its root: at 1,
    /// only the root spawns. None for Unlimited.
    pub max_depth: Option<NonZeroU32>,
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
    /// The text added to a child's system prompt after the product's, which
    /// it never replaces; none for no added text.
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
    /// The entry's name as Settings shows it, read with the settings; none
    /// once the entry is gone from the user's scope.
    pub provider_name: Option<String>,
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

/// The model options of a profile change: `--model`, `--provider`,
/// `--effort` and `--tier`. On an edit, a choice without a model changes
/// the effort or the tier of the profile's own model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelChoice {
    /// A model id the menu offers.
    pub model: Option<String>,
    /// The entry's name or id, needed when several entries offer the model.
    pub provider: Option<String>,
    /// An effort the model lists, or `off`.
    pub effort: Option<String>,
    /// A tier the model offers, or `default` for the vendor's default.
    pub tier: Option<String>,
}

/// The effort option that turns thinking off.
pub const EFFORT_OFF: &str = "off";

/// What a job says a round on `selection` runs on (`subagents.md` § What an
/// agent runs on): the entry's name `entry` and the model's name and Fast
/// tier from its catalog model `catalog`, the names the model menu shows;
/// the entry's id, the selection's model name and no Fast tier for what the
/// user's scope no longer has.
pub fn runs_on(
    selection: &ModelSelection,
    entry: Option<&str>,
    catalog: Option<&ProviderModel>,
) -> JobModel {
    let tier = selection.service_tier_id.as_deref();
    JobModel {
        provider: entry.unwrap_or(&selection.provider_id).to_owned(),
        model: catalog.map_or(&selection.model.name, |model| &model.display_name).clone(),
        effort: selection.thinking_effort().map(|effort| {
            if effort == THINKING_OFF { EFFORT_OFF } else { effort }.to_owned()
        }),
        fast: catalog.is_some_and(|model| {
            model
                .service_tiers
                .iter()
                .any(|listed| Some(listed.id.as_str()) == tier && listed.fast)
        }),
    }
}

/// The tier option that names the vendor's default tier.
pub const TIER_DEFAULT: &str = "default";

/// A new profile as `demi agent profiles create` gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProfileDraft {
    pub name: String,
    pub description: String,
    /// None for the parent's model.
    pub model: Option<ModelChoice>,
    pub instructions: Option<String>,
    pub can_spawn: bool,
    pub enabled: bool,
}

/// How `demi agent profiles edit` changes a profile's model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelEdit {
    /// `--model parent`: the parent's model again.
    Parent,
    Choose(ModelChoice),
}

/// The fields `demi agent profiles edit` changes; each it leaves none
/// stays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub model: Option<ModelEdit>,
    /// `Some(None)` removes the instructions.
    pub instructions: Option<Option<String>>,
    pub can_spawn: Option<bool>,
    pub enabled: Option<bool>,
}

/// Where the agent reads the user's subagent settings, at each spawn and
/// each `demi agent profiles` call, never once per tree, and changes the
/// user's profiles through the same path as the settings page, with its
/// checks and its words. A change reaches the next read.
pub trait SubagentSource {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>>;

    /// What a round on `selection` runs on, in the names the model menu
    /// shows now ([`runs_on`]).
    fn runs_on<'a>(&'a self, selection: &'a ModelSelection) -> LocalBoxFuture<'a, JobModel> {
        Box::pin(async move { runs_on(selection, None, None) })
    }

    /// Creates a profile; answers it as stored, or why it was refused.
    fn create_profile(
        &self,
        _profile: NewProfileDraft,
    ) -> LocalBoxFuture<'_, Result<Profile, String>> {
        Box::pin(async { Err(FIXED.to_owned()) })
    }

    /// Changes the profile `name`; answers it as stored, or why it was
    /// refused.
    fn edit_profile<'a>(
        &'a self,
        _name: &'a str,
        _edit: ProfileEdit,
    ) -> LocalBoxFuture<'a, Result<Profile, String>> {
        Box::pin(async { Err(FIXED.to_owned()) })
    }

    /// Deletes the profile `name`, or answers why it was not.
    fn delete_profile<'a>(&'a self, _name: &'a str) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async { Err(FIXED.to_owned()) })
    }
}

/// Why a source whose settings never change refuses a change.
const FIXED: &str = "the subagent settings of this product cannot be changed";

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

/// What a context source tells the node: its block's text, and for the
/// instructions source what the text holds (`instructions.md` § What the
/// card lists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextAnswer {
    pub text: String,
    pub instructions: Vec<InstructionEntry>,
}

impl From<String> for ContextAnswer {
    fn from(text: String) -> Self {
        Self {
            text,
            instructions: Vec::new(),
        }
    }
}

/// One source of what the model must learn before a request (`runtime.md`
/// § Context), such as the conversation's execution context, the
/// instructions or a plugin.
pub trait ContextSource {
    /// The name its blocks record: `execution`, `instructions`, or a
    /// plugin's id.
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
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>>;
}
