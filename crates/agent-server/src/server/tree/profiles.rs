//! The user's subagent settings as a tree reads them.
//!
//! This module holds the checks a spawn makes against the settings and the
//! conversation's Workflow menu choice as they are at that moment, and `demi
//! agent profiles`.
//!
//! Nothing is kept between two reads, so a change in settings, in the choice or
//! in a workflow's file reaches the next spawn of every tree, an open one
//! included. A child keeps what it was spawned with, so a profile changed or
//! deleted afterwards changes no child.
//!
//! A profile never replaces the product's prompt (the child is always Demi) and
//! never changes the commands. A spawn without `--profile` takes the inherit
//! profile, which exists whether or not the user has profiles.
//!
//! The checks run in the order of `Tree::spawn_setup`. The reasons behind them:
//!
//! - Nothing falls back. A profile whose model settings are missing (entry
//!   gone, model dropped, effort or tier no longer offered) is unavailable, and
//!   a spawn fails naming what is missing. It never runs on the parent's model
//!   or another effort or tier, so a spawn never runs on something the user did
//!   not choose. A disabled profile is the user's choice, so one that is also
//!   unavailable is reported disabled.
//! - The switch blocks spawns only. Running children go on, and `send`,
//!   `abort`, `resume`, `list` and `show` keep working, because they act on
//!   agents that exist. A resume continues a child the parent already has.
//! - The limits are the user's across all their conversations, because they
//!   bound what the user runs at once (tokens, the Hosts' load), not what one
//!   conversation does. A tree with a live subagent is never evicted, so every
//!   live one is counted. A limit lowered below what is live ends nothing.
//! - A workflow chosen for the conversation replaces the user's profiles and
//!   the switch.
//! - `profiles list` reads the settings at call time and succeeds while
//!   subagents are off. The profiles are in neither the system prompt nor a
//!   context block: the prompt is rendered once per tree and could not follow
//!   an edit, and a block would put every edit into every conversation's
//!   transcript, spawning or not.
//! - `create`, `edit` and `delete` change a setting every conversation reads,
//!   so each needs the user's permission in each conversation. They go through
//!   the settings page's path, with its checks and words.

use std::{num::NonZeroU32, rc::Rc};

use demi_agent_tools::{
    ChosenWorkflow, ConversationProfiles, EFFORT_OFF, HostResolver, NewProfileDraft, Profile,
    ProfileEdit, SubagentSettings, Unavailable, WorkflowChild, WorkflowListing,
};
use demi_shared_types::{
    ModelSelection, THINKING_OFF, WorkflowChoice, context_word, offers_context,
    output::{Cell, Fields, Table},
};
use futures_util::future::join_all;
use schemars::JsonSchema;
use serde::Serialize;

use super::Tree;
use crate::{Node, server::AgentServer};

/// Why a spawn fails while the user has subagents off.
const SUBAGENTS_OFF: &str = "subagents are turned off in settings; do this work in this session, or ask the user to turn subagents on in Settings > Subagents.";

/// Why a spawn fails in a conversation whose Workflow menu says No
/// Subagents.
const OFF_HERE: &str = "subagents are turned off for this conversation; do this work in this session, or have Global Workflow or a workflow picked: the user picks one in the composer's Workflow menu, or you pick one with demi agent workflows use global or use <id> once the user allows it.";

/// What a spawn makes its child of: the profile's settings, or the
/// parent's for the inherit profile.
pub(super) struct Setup {
    pub(super) model: ModelSelection,
    /// The instructions the profile adds to the child's system prompt; none
    /// when it adds none, and for the inherit profile, whose child copies
    /// its parent's.
    pub(super) instructions: Option<String>,
    /// The Context the profile chose, in tokens, in place of the user's
    /// limit; none for the user's. With the parent's model, the parent's.
    pub(super) context_limit: Option<u32>,
    pub(super) can_spawn: bool,
    /// The limits the settings set at that moment, which the spawn checks
    /// next ([`Limits`]).
    pub(super) limits: Limits,
}

/// The user's two limits on subagents: the live
/// subagents of all the user's conversations together and each tree's
/// depth, each none for Unlimited.
#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    pub(super) max_live: Option<NonZeroU32>,
    pub(super) max_depth: Option<NonZeroU32>,
}

impl Limits {
    fn of(settings: &SubagentSettings) -> Self {
        Self {
            max_live: settings.max_live,
            max_depth: settings.max_depth,
        }
    }
}

/// `demi agent profiles list --json`: whether this conversation's agents
/// may spawn, its Workflow menu choice, and the profiles they may spawn
/// with: each enabled profile of the user's, or the chosen workflow's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ProfileListing {
    /// False while the user has subagents off with Global Workflow chosen, with No
    /// Subagents chosen, and while the chosen workflow is unavailable.
    enabled: bool,
    choice: WorkflowChoice,
    /// The chosen workflow's name; absent without one, and while it is
    /// unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow: Option<String>,
    /// Why the chosen workflow cannot be used now; absent while it can.
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable: Option<String>,
    profiles: Vec<ListedProfile>,
}

/// One profile of `demi agent profiles list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ListedProfile {
    name: String,
    /// The workflow the profile is of; null for the user's settings.
    workflow: Option<String>,
    /// The model id its children run; null for the parent's, and for a
    /// workflow's profile whose model is unavailable.
    model: Option<String>,
    /// The effort its model runs at, or `off`; null for the parent's, and
    /// for a model that lists none.
    effort: Option<String>,
    /// Whether its children run the parent's model.
    inherited: bool,
    description: String,
    /// The message a spawn with the profile fails with now, which says
    /// what is missing; null while it is available.
    unavailable: Option<String>,
}

/// An effort as `profiles` prints it: Demi's word for thinking off.
fn effort_word(effort: &str) -> String {
    if effort == THINKING_OFF { EFFORT_OFF } else { effort }.to_owned()
}

impl<H: HostResolver> Tree<H> {
    /// What a spawn by `owner` with the profile `name` makes its child of.
    ///
    /// `name` is none for the inherit profile. The checks run against the
    /// user's current settings and the conversation's choice. The first that
    /// fails ends the spawn before anything is created. In order, they require
    /// that:
    ///
    /// 1. subagents are on;
    /// 2. the name is known;
    /// 3. the profile is enabled;
    /// 4. its model settings make a selection.
    ///
    /// A workflow's profiles replace the user's, whatever the user's switch is.
    /// The limits the spawn checks next come with the answer.
    pub(super) async fn spawn_setup(
        &self,
        server: &AgentServer<H>,
        owner: &Node<H>,
        name: Option<&str>,
    ) -> Result<Setup, String> {
        let settings = read(server).await?;
        let limits = Limits::of(&settings);
        // The parent's model comes with the parent's Context, for the
        // inherit profile and for a profile without a model of its own
        // alike.
        let parents = || (owner.session().model(), owner.record().context_limit);
        let inherit = || {
            let (model, context_limit) = parents();
            Setup {
                model,
                instructions: None,
                context_limit,
                can_spawn: true,
                limits,
            }
        };
        match self.conversation_profiles(server).await? {
            ConversationProfiles::Off => Err(OFF_HERE.to_owned()),
            ConversationProfiles::Unavailable { id, reason } => Err(format!(
                "this conversation's workflow \"{id}\" is unavailable: {reason}; do this work in this session, or have another picked: the user picks one in the composer's Workflow menu, or you pick one with demi agent workflows use <id> once the user allows it."
            )),
            ConversationProfiles::Workflow(workflow) => {
                let Some(name) = name else {
                    return Ok(inherit());
                };
                let child = workflow
                    .profiles
                    .iter()
                    .find(|child| child.name == name)
                    .ok_or_else(|| unknown_in(name, &workflow))?;
                let (model, context_limit) = match &child.model {
                    None => parents(),
                    Some(Ok(selection)) => (selection.clone(), child.context_limit),
                    Some(Err(missing)) => {
                        return Err(format!("profile \"{name}\" is unavailable: {missing}"));
                    }
                };
                Ok(Setup {
                    model,
                    instructions: child.instructions.clone(),
                    context_limit,
                    can_spawn: child.can_spawn,
                    limits,
                })
            }
            ConversationProfiles::Settings => {
                if !settings.enabled {
                    return Err(SUBAGENTS_OFF.to_owned());
                }
                let Some(name) = name else {
                    return Ok(inherit());
                };
                let profile = settings
                    .profiles
                    .iter()
                    .find(|profile| profile.name == name)
                    .ok_or_else(|| unknown(name, &settings))?;
                if !profile.enabled {
                    return Err(format!(
                        "profile \"{name}\" is disabled in settings; omit --profile to inherit the parent, or ask the user to enable it in Settings > Subagents."
                    ));
                }
                let (model, context_limit) = match &profile.model {
                    Some(model) => (
                        server
                            .deps
                            .providers
                            .profile_selection(model)
                            .await
                            .map_err(|missing| unavailable(name, &missing))?,
                        profile.context_limit,
                    ),
                    None => parents(),
                };
                Ok(Setup {
                    model,
                    instructions: profile.instructions.clone(),
                    context_limit,
                    can_spawn: profile.can_spawn,
                    limits,
                })
            }
        }
    }

    /// `demi agent workflows list`: the workflows of the conversation's
    /// working directory, as the product holds them, and its choice.
    pub(crate) async fn workflows(&self) -> Result<WorkflowListing, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        server.deps.subagents.workflows(self.root.id()).await
    }

    /// `demi agent workflows use`: makes `choice` the conversation's
    /// Workflow menu choice; answers the chosen workflow's name.
    pub(crate) async fn choose_workflow(&self, choice: WorkflowChoice) -> Result<Option<String>, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        server.deps.subagents.choose_workflow(self.root.id(), choice).await
    }

    /// The text of the file `path`, read against `directory`, the invoking
    /// shell's, which `demi agent workflows check` parses.
    pub(crate) async fn workflow_file(&self, directory: &str, path: &str) -> Result<String, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        server
            .deps
            .subagents
            .read_workflow_file(self.root.id(), directory, path)
            .await
    }

    /// The limits the user's settings set now, which a resume checks.
    pub(super) async fn limits(&self, server: &AgentServer<H>) -> Result<Limits, String> {
        Ok(Limits::of(&read(server).await?))
    }

    /// The profiles this conversation's spawns use now.
    async fn conversation_profiles(
        &self,
        server: &AgentServer<H>,
    ) -> Result<ConversationProfiles, String> {
        server
            .deps
            .subagents
            .conversation_profiles(self.root.id())
            .await
            .map_err(|error| format!("the conversation's workflow cannot be read: {error}"))
    }

    /// `demi agent profiles list`, from the user's settings and the
    /// conversation's choice now.
    ///
    /// It lists:
    ///
    /// - the chosen workflow's profiles in its order;
    /// - or, with Global Workflow and subagents on, each enabled profile of the
    ///   user's in name order;
    /// - an unavailable one with what is missing.
    pub(crate) async fn profiles(&self) -> Result<ProfileListing, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let off = |choice| ProfileListing {
            enabled: false,
            choice,
            workflow: None,
            unavailable: None,
            profiles: Vec::new(),
        };
        match self.conversation_profiles(&server).await? {
            ConversationProfiles::Off => Ok(off(WorkflowChoice::None)),
            ConversationProfiles::Unavailable { id, reason } => Ok(ProfileListing {
                unavailable: Some(reason),
                ..off(WorkflowChoice::Workflow { id })
            }),
            ConversationProfiles::Workflow(workflow) => {
                let profiles = workflow
                    .profiles
                    .iter()
                    .map(|child| ListedProfile {
                        name: child.name.clone(),
                        workflow: Some(workflow.name.clone()),
                        model: match &child.model {
                            Some(Ok(selection)) => Some(selection.model.id.clone()),
                            None | Some(Err(_)) => None,
                        },
                        effort: match &child.model {
                            Some(Ok(selection)) => selection.thinking_effort().map(effort_word),
                            None | Some(Err(_)) => None,
                        },
                        inherited: child.model.is_none(),
                        description: child.description.clone(),
                        unavailable: match &child.model {
                            Some(Err(missing)) => {
                                Some(format!("profile \"{}\" is unavailable: {missing}", child.name))
                            }
                            None | Some(Ok(_)) => None,
                        },
                    })
                    .collect();
                Ok(ProfileListing {
                    enabled: true,
                    choice: WorkflowChoice::Workflow { id: workflow.id },
                    workflow: Some(workflow.name),
                    unavailable: None,
                    profiles,
                })
            }
            ConversationProfiles::Settings => {
                let settings = read(&server).await?;
                if !settings.enabled {
                    return Ok(off(WorkflowChoice::Default));
                }
                let enabled = settings.profiles.iter().filter(|profile| profile.enabled);
                let profiles = join_all(enabled.map(|profile| listed(&server, profile))).await;
                Ok(ProfileListing {
                    enabled: true,
                    choice: WorkflowChoice::Default,
                    workflow: None,
                    unavailable: None,
                    profiles,
                })
            }
        }
    }
}

/// `demi agent profiles show --json` and the answer of a profile change:
/// every field of one profile, enabled or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileAnswer {
    name: String,
    /// The workflow the profile is of; null for the user's settings.
    workflow: Option<String>,
    description: String,
    /// Null for the parent's model.
    model: Option<ShownModel>,
    /// The context window its children use, as `--context` names it:
    /// `default` for the user's limit, `full`, or a limit such as `500K`.
    context: String,
    /// The tokens of that window; null for the user's limit, and while the
    /// model's window is unknown.
    context_tokens: Option<u32>,
    /// Null for none.
    instructions: Option<String>,
    /// Whether its children may spawn children of their own.
    spawning: bool,
    enabled: bool,
    /// The message a spawn with it fails with now, which says what is
    /// missing; null while its model settings make a selection.
    unavailable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ShownModel {
    /// The provider entry's id.
    provider: String,
    /// The entry's name as Settings shows it; null once the entry is gone,
    /// and for a workflow's profile.
    provider_name: Option<String>,
    model: String,
    /// An effort the model lists, or `off`; null only for a model that
    /// lists none.
    effort: Option<String>,
    /// Null for the vendor's default tier.
    tier: Option<String>,
}

/// `demi agent profiles delete --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct Deleted {
    name: String,
    deleted: bool,
}

impl Deleted {
    pub(crate) fn render(&self) -> String {
        format!("deleted profile {}\n", self.name)
    }
}

impl<H: HostResolver> Tree<H> {
    /// `demi agent profiles show`: the user's profile `name`, enabled or
    /// not, checked now.
    pub(crate) async fn profile(&self, name: &str) -> Result<ProfileAnswer, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        // The profiles `list` lists: the chosen workflow's in place of the
        // user's (`crates/backend-user-shard/src/conversation/workflows.rs`).
        if let ConversationProfiles::Workflow(workflow) = self.conversation_profiles(&server).await? {
            let child = workflow
                .profiles
                .iter()
                .find(|child| child.name == name)
                .ok_or_else(|| unknown_in(name, &workflow))?;
            return Ok(workflow_answer(&workflow, child));
        }
        let settings = read(&server).await?;
        let profile = settings
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| no_profile(name, &settings))?;
        Ok(answer(&server, profile).await)
    }

    /// `demi agent profiles create`: the profile as stored.
    ///
    /// While this conversation runs a workflow, it also answers why a spawn
    /// here cannot use the profile: a workflow's profiles replace the user's
    /// (`crates/backend-user-shard/src/conversation/workflows.rs`).
    pub(crate) async fn create_profile(
        &self,
        draft: NewProfileDraft,
    ) -> Result<(ProfileAnswer, Option<String>), String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        // Read before the profile is saved, so that what the warning needs
        // cannot fail a creation that succeeded.
        let workflow = match self.conversation_profiles(&server).await? {
            ConversationProfiles::Workflow(workflow) => Some(workflow.name),
            ConversationProfiles::Off
            | ConversationProfiles::Unavailable { .. }
            | ConversationProfiles::Settings => None,
        };
        let created = server.deps.subagents.create_profile(draft).await?;
        let warning = workflow.map(|workflow| format!(
            "this conversation runs workflow {workflow}, so demi agent spawn here takes only its profiles; \"{}\" is for the user's other conversations",
            created.name
        ));
        Ok((answer(&server, &created).await, warning))
    }

    /// `demi agent profiles edit`: the profile as stored.
    pub(crate) async fn edit_profile(
        &self,
        name: &str,
        edit: ProfileEdit,
    ) -> Result<ProfileAnswer, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let changed = server.deps.subagents.edit_profile(name, edit).await?;
        Ok(answer(&server, &changed).await)
    }

    /// `demi agent profiles delete`.
    pub(crate) async fn delete_profile(&self, name: &str) -> Result<Deleted, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        server.deps.subagents.delete_profile(name).await?;
        Ok(Deleted {
            name: name.to_owned(),
            deleted: true,
        })
    }
}

impl ProfileAnswer {
    /// The text `profiles show` and a change print: a field per line, and
    /// the instructions in full last.
    pub(crate) fn render(&self) -> String {
        let model = self.model.as_ref();
        let mut fields = Fields::new();
        fields
            .field("Name", self.name.as_str())
            .field("Source", self.workflow.as_deref().unwrap_or("user"))
            .field("Description", self.description.as_str())
            .field("Status", status(self.unavailable.is_some()))
            .field("Unavailable", self.unavailable.as_deref())
            .field("Model", model.map(|model| model.model.as_str()))
            .field(
                "Provider",
                model.map(|model| model.provider_name.as_deref().unwrap_or(&model.provider)),
            )
            .field("Effort", model.and_then(|model| model.effort.as_deref()))
            .field("Tier", model.and_then(|model| model.tier.as_deref()))
            .field("Context (tokens)", self.context_tokens)
            .field("Spawning", self.spawning)
            .field("Enabled", self.enabled)
            .text("Instructions", self.instructions.as_deref().unwrap_or_default());
        fields.render()
    }
}

/// A profile's status as `list` and `show` print it.
fn status(unavailable: bool) -> &'static str {
    if unavailable { "unavailable" } else { "available" }
}

/// The profile `child` of the chosen `workflow` as `show` gives it.
fn workflow_answer(workflow: &ChosenWorkflow, child: &WorkflowChild) -> ProfileAnswer {
    let selection = child.model.as_ref().and_then(|model| model.as_ref().ok());
    ProfileAnswer {
        name: child.name.clone(),
        workflow: Some(workflow.name.clone()),
        description: child.description.clone(),
        model: selection.map(|selection| ShownModel {
            provider: selection.provider_id.clone(),
            provider_name: None,
            model: selection.model.id.clone(),
            effort: selection.thinking_effort().map(effort_word),
            tier: selection.service_tier_id.clone(),
        }),
        context: child.context_limit.map_or_else(|| "default".to_owned(), context_word),
        context_tokens: child.context_limit,
        instructions: child.instructions.clone(),
        spawning: child.can_spawn,
        enabled: true,
        unavailable: match &child.model {
            Some(Err(missing)) => Some(format!("profile \"{}\" is unavailable: {missing}", child.name)),
            None | Some(Ok(_)) => None,
        },
    }
}

/// `profile` as `show` gives it, checked now.
async fn answer<H: HostResolver>(server: &AgentServer<H>, profile: &Profile) -> ProfileAnswer {
    let selection = match &profile.model {
        Some(model) => Some(server.deps.providers.profile_selection(model).await),
        None => None,
    };
    let window = match &selection {
        Some(Ok(selection)) => Some(selection.model.context_window),
        _ => None,
    };
    // What applies now: a stored choice the window no longer offers is the
    // full window, as for a session.
    let (context, context_tokens) = match (profile.context_limit, window) {
        (None, _) => ("default".to_owned(), None),
        (Some(tokens), Some(window)) if !offers_context(window, tokens) || tokens == window => {
            ("full".to_owned(), Some(window).filter(|window| *window > 0))
        }
        (Some(tokens), _) => (context_word(tokens), Some(tokens)),
    };
    let missing = selection.and_then(Result::err);
    ProfileAnswer {
            name: profile.name.clone(),
            workflow: None,
            description: profile.description.clone(),
            model: profile.model.as_ref().map(|model| ShownModel {
                provider: model.provider_id.clone(),
                provider_name: model.provider_name.clone(),
                model: model.model_id.clone(),
                effort: model.thinking_effort.as_deref().map(effort_word),
                tier: model.service_tier_id.clone(),
            }),
            context,
            context_tokens,
            instructions: profile.instructions.clone(),
            spawning: profile.can_spawn,
            enabled: profile.enabled,
            unavailable: missing.map(|missing| unavailable(&profile.name, &missing)),
    }
}

/// Why `show` finds no profile `name`: with every profile's name, enabled
/// or not, since `show` reads either.
fn no_profile(name: &str, settings: &SubagentSettings) -> String {
    let names: Vec<&str> = settings
        .profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect();
    if names.is_empty() {
        return format!("no profile \"{name}\"; the user has no subagent profile");
    }
    format!(
        "no profile \"{name}\"; the user's profiles are {}",
        names.join(", ")
    )
}

impl ProfileListing {
    /// What makes each unavailable profile so, for stderr.
    pub(crate) fn unavailable(&self) -> impl Iterator<Item = &str> {
        self.profiles.iter().filter_map(|profile| profile.unavailable.as_deref())
    }

    /// Why no agent of the conversation may spawn now, or with no profile what a
    /// spawn does, which `list` tells on stderr; none while profiles are listed.
    pub(crate) fn note(&self) -> Option<String> {
        match &self.choice {
            WorkflowChoice::Default if !self.enabled => Some(
                "subagents are turned off in settings; demi agent spawn fails until the user turns them on".to_owned(),
            ),
            WorkflowChoice::None => Some(
                "subagents are turned off for this conversation; demi agent spawn fails until Global Workflow or a workflow is picked in the composer's Workflow menu, or with demi agent workflows use global or use <id> once the user allows it".to_owned(),
            ),
            WorkflowChoice::Workflow { id } if !self.enabled => Some(format!(
                "this conversation's workflow \"{id}\" is unavailable: {}; demi agent spawn fails until another is picked in the composer's Workflow menu, or with demi agent workflows use <id> once the user allows it",
                self.unavailable.as_deref().unwrap_or_default()
            )),
            _ if self.profiles.is_empty() => Some(match &self.workflow {
                None => "the user has no enabled subagent profile; demi agent spawn without --profile inherits the parent".to_owned(),
                Some(name) => format!("workflow {name} has no profiles; demi agent spawn without --profile inherits the parent"),
            }),
            WorkflowChoice::Default | WorkflowChoice::Workflow { .. } => None,
        }
    }

    /// The profiles `list --json` gives.
    pub(crate) fn profiles(&self) -> &[ListedProfile] {
        &self.profiles
    }

    /// The table `demi agent profiles list` prints: where each profile
    /// comes from, the workflow's name or `user`, and the model and effort
    /// its children run, so a caller choosing one sees what each runs.
    pub(crate) fn render(&self) -> String {
        let mut table = Table::new(["name", "source", "model", "effort", "description"]);
        for profile in &self.profiles {
            // An unavailable profile runs no model now: its reason is a
            // warning, never a row that names the model it cannot reach.
            let model = match &profile.model {
                _ if profile.unavailable.is_some() => None,
                _ if profile.inherited => Some("inherited"),
                model => model.as_deref(),
            };
            let effort = profile.effort.as_deref().filter(|_| profile.unavailable.is_none());
            table.row::<Cell>([
                profile.name.as_str().into(),
                profile.workflow.as_deref().unwrap_or("user").into(),
                model.into(),
                effort.into(),
                profile.description.as_str().into(),
            ]);
        }
        table.render()
    }
}

/// The user's subagent settings now.
async fn read<H: HostResolver>(server: &AgentServer<H>) -> Result<SubagentSettings, String> {
    server
        .deps
        .subagents
        .current()
        .await
        .map_err(|error| format!("the subagent settings cannot be read: {error}"))
}

/// An enabled profile as `demi agent profiles list` lists it, checked now.
async fn listed<H: HostResolver>(server: &Rc<AgentServer<H>>, profile: &Profile) -> ListedProfile {
    let missing = match &profile.model {
        Some(model) => server.deps.providers.profile_selection(model).await.err(),
        None => None,
    };
    let model = profile.model.as_ref();
    ListedProfile {
        name: profile.name.clone(),
        workflow: None,
        model: model.map(|model| model.model_id.clone()),
        effort: model.and_then(|model| model.thinking_effort.as_deref()).map(effort_word),
        inherited: model.is_none(),
        description: profile.description.clone(),
        unavailable: missing.map(|missing| unavailable(&profile.name, &missing)),
    }
}

/// Why a spawn names no profile of the user's: with the enabled profiles'
/// names, so a model that guessed or remembered an old name learns the
/// current ones.
fn unknown(name: &str, settings: &SubagentSettings) -> String {
    let names: Vec<&str> = settings
        .profiles
        .iter()
        .filter(|profile| profile.enabled)
        .map(|profile| profile.name.as_str())
        .collect();
    if names.is_empty() {
        return format!(
            "unknown profile \"{name}\"; the user has no enabled profile. Omit --profile to inherit the parent."
        );
    }
    format!(
        "unknown profile \"{name}\"; the user's profiles are {}. Run `demi agent profiles list` for when to use each, or omit --profile to inherit the parent.",
        names.join(", ")
    )
}

/// Why a spawn names no profile of the conversation's workflow: with the
/// workflow's names.
fn unknown_in(name: &str, workflow: &ChosenWorkflow) -> String {
    let names: Vec<&str> = workflow
        .profiles
        .iter()
        .map(|child| child.name.as_str())
        .collect();
    if names.is_empty() {
        return format!(
            "unknown profile \"{name}\"; this conversation's workflow {} has no profiles. Omit --profile to inherit the parent.",
            workflow.name
        );
    }
    format!(
        "unknown profile \"{name}\"; the profiles of this conversation's workflow {} are {}. Run `demi agent profiles list` for when to use each, or omit --profile to inherit the parent.",
        workflow.name,
        names.join(", ")
    )
}

/// Why a spawn with the unavailable profile `name` fails.
fn unavailable(name: &str, missing: &Unavailable) -> String {
    format!("profile \"{name}\" is unavailable: {missing}")
}
