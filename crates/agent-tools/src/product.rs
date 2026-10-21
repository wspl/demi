//! What a product supplies for its agents besides the data the agent server is
//! given.
//!
//! The product supplies:
//!
//! - the commands a tree opens with;
//! - the user's subagent settings each spawn reads;
//! - the Host a node's shell tools reach;
//! - the context sources asked before each request.

use std::num::NonZeroU32;
use std::rc::Rc;

use demi_conversation_socket_protocol::JobModel;
use demi_agent_store::Placement;
use demi_host_interface::{CommandSet, Host, HostError, HostErrorKind};
use demi_shared_types::{
    ContextChoice, InstructionEntry, ModelSelection, NodeId, ProviderModel, THINKING_OFF, TurnId,
    WorkflowChoice,
};
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::Serialize;

/// The commands every node of a tree starts from, which a tree takes when
/// it opens and keeps until it closes; `revision` tells two toolsets with
/// other commands apart.
#[derive(Clone)]
pub struct Toolset {
    pub commands: Rc<CommandSet>,
    pub revision: Rc<str>,
}

/// Where a tree that opens takes its toolset from: the product's current one,
/// which may change while the server runs.
///
/// The server composes a revision once and every node holds that one set, so it
/// asks for the revision first and for the commands only when it holds none of
/// that revision.
pub trait ToolsetSource {
    /// The revision of the toolset [`current`](Self::current) would give
    /// now, read without composing it.
    fn revision(&self) -> LocalBoxFuture<'_, Result<Rc<str>, String>>;
    /// The toolset the product gives now, composed afresh.
    fn current(&self) -> LocalBoxFuture<'_, Result<Toolset, String>>;
}

/// A toolset that never changes.
impl ToolsetSource for Toolset {
    fn revision(&self) -> LocalBoxFuture<'_, Result<Rc<str>, String>> {
        let revision = self.revision.clone();
        Box::pin(async move { Ok(revision) })
    }

    fn current(&self) -> LocalBoxFuture<'_, Result<Toolset, String>> {
        let toolset = self.clone();
        Box::pin(async move { Ok(toolset) })
    }
}

/// The user's subagent settings at one moment
/// (`crates/agent-server/src/server/tree/profiles.rs`): the Subagents switch,
/// the limits and the user's profiles, in name order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSettings {
    /// Whether the user has subagents on.
    pub enabled: bool,
    /// How many subagents may be live at once across all the user's
    /// conversations, at every depth together; none for Unlimited.
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
    /// The tokens of its own Context, which its children use in place of
    /// the user's limit; none for the user's limit, and without a model.
    pub context_limit: Option<u32>,
    /// The text added to a child's system prompt after the product's, which
    /// it never replaces; none for no added text.
    pub instructions: Option<String>,
    /// Whether its children may spawn children of their own.
    pub can_spawn: bool,
    /// Whether agents may use it now.
    pub enabled: bool,
}

/// A profile's model settings (`crates/shared-types/src/catalog.rs`): the
/// provider entry, the model, the thinking effort and the service tier the user
/// chose.
///
/// The product builds a child's model selection from them at each spawn.
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
/// profile unavailable (`crates/agent-server/src/server/tree/profiles.rs`). Each says
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

/// The profiles a conversation's spawns use now
/// (`crates/backend-user-shard/src/conversation/workflows.rs`): its Workflow
/// menu choice, with the chosen workflow as its file reads at this moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationProfiles {
    /// Global Workflow: the user's Subagents settings.
    Settings,
    /// No Subagents: subagents are off for this conversation.
    Off,
    /// The profiles of the chosen workflow, in place of the user's.
    Workflow(ChosenWorkflow),
    /// The chosen workflow cannot be used now: its file is gone or does not
    /// read, as `reason` says.
    Unavailable { id: String, reason: String },
}

/// A workflow a conversation chose, as its file reads now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChosenWorkflow {
    pub id: String,
    /// What the menu shows.
    pub name: String,
    /// Its profiles, in the file's order.
    pub profiles: Vec<WorkflowChild>,
}

/// One profile of a chosen workflow, its model resolved against the user's
/// provider entries now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowChild {
    /// The `--profile` value.
    pub name: String,
    pub description: String,
    /// Added to the child's system prompt after the product's.
    pub instructions: Option<String>,
    pub can_spawn: bool,
    /// None for the parent's model; otherwise the selection its model id
    /// makes in the first entry that offers it, or what is missing or not
    /// offered, its Context included, which makes the profile unavailable.
    pub model: Option<Result<ModelSelection, String>>,
    /// The tokens of its own Context, which the model offers; none for the
    /// user's limit.
    pub context_limit: Option<u32>,
}

/// `demi agent workflows list`: the conversation's choice, and the
/// workflows of its working directory as the menu lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct WorkflowListing {
    pub chosen: WorkflowChoice,
    /// The directory that holds them, `.demi/workflows` of the nearest
    /// directory that has one; null when none has.
    pub directory: Option<String>,
    /// Why that directory cannot be used, as the Host could not read it; null
    /// while it can.
    pub unavailable: Option<String>,
    /// By id.
    pub workflows: Vec<ListedWorkflow>,
}

/// A workflow file of the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListedWorkflow {
    pub id: String,
    /// What the menu shows; null for a file that cannot be used.
    pub name: Option<String>,
    /// The file on the Host.
    pub path: String,
    /// Why it cannot be used: the parser's whole error, or why the Host could
    /// not read it; null while it can.
    pub unavailable: Option<String>,
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

/// What a job says a round on `selection` runs on.
///
/// With the user's scope still holding the entry and model, it is the entry's
/// name `entry` and the model's name and Fast tier from its catalog model
/// `catalog`, the names the model menu shows. For what the user's scope no
/// longer has, it is the entry's id, the selection's model name and no Fast
/// tier.
pub fn runs_on(
    selection: &ModelSelection,
    entry: Option<&str>,
    catalog: Option<&ProviderModel>,
) -> JobModel {
    let tier = selection.service_tier_id.as_deref();
    JobModel {
        provider_id: selection.provider_id.clone(),
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
    /// `--context`; none for the user's limit.
    pub context: Option<ContextChoice>,
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
    /// `--context`.
    pub context: Option<ContextChoice>,
    /// `Some(None)` removes the instructions.
    pub instructions: Option<Option<String>>,
    pub can_spawn: Option<bool>,
    pub enabled: Option<bool>,
}

/// Where the agent reads the user's subagent settings, at each spawn and each
/// `demi agent profiles` call, never once per tree.
///
/// It also changes the user's profiles through the same path as the settings
/// page, with its checks and its words. A change reaches the next read.
pub trait SubagentSource {
    fn current(&self) -> LocalBoxFuture<'_, Result<SubagentSettings, String>>;

    /// What rounds on `selections` run on, in their order, in the names the
    /// model menu shows now ([`runs_on`]).
    ///
    /// One call answers a whole tree, so a tree that opens reads the names
    /// once, not once per agent.
    fn runs_on<'a>(&'a self, selections: &'a [ModelSelection]) -> LocalBoxFuture<'a, Vec<JobModel>> {
        Box::pin(async move {
            selections
                .iter()
                .map(|selection| runs_on(selection, None, None))
                .collect()
        })
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

    /// The profiles the spawns of the conversation `root` use now, from its
    /// Workflow menu choice, the chosen workflow's file read as it is.
    fn conversation_profiles<'a>(
        &'a self,
        _root: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<ConversationProfiles, String>> {
        Box::pin(async { Ok(ConversationProfiles::Settings) })
    }

    /// The workflows of the conversation `root`'s working directory, as the
    /// product holds them, and its choice.
    fn workflows<'a>(&'a self, _root: &'a NodeId) -> LocalBoxFuture<'a, Result<WorkflowListing, String>> {
        Box::pin(async { Err(NO_WORKFLOWS.to_owned()) })
    }

    /// Makes `choice` the conversation `root`'s Workflow menu choice through
    /// the menu's own change, so the menu shows it at once; answers the
    /// chosen workflow's name, none for Global Workflow and No Subagents. A
    /// workflow the project does not have, or that cannot be used, is refused
    /// with why, and the choice stays.
    fn choose_workflow<'a>(
        &'a self,
        _root: &'a NodeId,
        _choice: WorkflowChoice,
    ) -> LocalBoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async { Err(NO_WORKFLOWS.to_owned()) })
    }

    /// The text of the file `path`, read against `directory`, on the
    /// conversation `root`'s Host, which `demi agent workflows check` parses.
    fn read_workflow_file<'a>(
        &'a self,
        _root: &'a NodeId,
        _directory: &'a str,
        _path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async { Err(NO_WORKFLOWS.to_owned()) })
    }
}

/// Why a product without a Host to read workflows from lists none.
const NO_WORKFLOWS: &str = "this product reads no workflows";

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
    /// A child's working directory now (`crates/agent-server/src/node/mod.rs`);
    /// none for the root, whose directory is the conversation's.
    pub cwd: Option<&'a str>,
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
    /// execution target (`crates/backend-host-access/src/access.rs`).
    ///
    /// Two answers for the same target have equal keys, so a node keeps one
    /// shell environment per target it used. By default there is none, and a
    /// shell tool's call fails.
    async fn host(&self, _context: NodeContext<'_>) -> Result<Rc<Self::Host>, HostError> {
        Err(HostError::new(
            HostErrorKind::Unavailable,
            "this agent runs no shell tools",
        ))
    }

    /// Where the conversation works now: its primary Host's device and its
    /// directory there, which a child's working directory is read against
    /// (`crates/agent-server/src/node/mod.rs`).
    ///
    /// By default its Host's key and the directory the Host starts its work in
    /// ([`Host::default_cwd`]).
    async fn placement(&self, context: NodeContext<'_>) -> Result<Placement, HostError> {
        let host = self.host(context).await?;
        Ok(Placement {
            device: Some(host.key().to_string()),
            directory: host.default_cwd().to_owned(),
        })
    }
}

/// What a context source tells the node: its block's text, and for the
/// instructions source what the text holds.
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

/// One source of what the model must learn before a request, such as the
/// conversation's execution context, the instructions or a plugin.
///
/// What the model must learn between its requests, and what differs by user or
/// conversation, reaches it as `context` blocks, never through the system
/// prompt. For example, when the user moves the conversation from the Cloud to
/// a laptop, the execution context source answers the new Host before the
/// node's next request.
///
/// The session asks each source before every request, in the product's order,
/// appends each answer as one block that names its source and saves it at once,
/// so a request never carries a context its transcript does not hold.
///
/// A source sees only the blocks the model still receives, those from the last
/// compaction boundary on, so after a compaction it tells the model again what
/// the summary may have left out.
pub trait ContextSource {
    /// The name its blocks record: `execution`, `instructions`,
    /// `side_questions`, or a plugin's id.
    fn name(&self) -> &str;

    /// What the node must learn now, or nothing.
    ///
    /// `seen` is the text of the source's own blocks the model receives, oldest
    /// first, and `turn` the input turn the request belongs to. A failure adds
    /// no block; the source is asked again before the next request.
    fn context<'a>(
        &'a self,
        node: NodeContext<'a>,
        turn: &'a TurnId,
        seen: &'a [&'a str],
    ) -> LocalBoxFuture<'a, Result<Option<ContextAnswer>, String>>;
}
