//! The user's subagent settings as a tree reads them (`subagents.md`
//! § Profiles, § Limits, § Managing profiles, § Workflows of a project):
//! the checks a spawn makes against the settings and the conversation's
//! Workflow menu choice as they are at that moment, and `demi agent
//! profiles`. Nothing here is kept between two reads, so a change in
//! settings, in the choice or in the workflow's file reaches the next spawn
//! of every tree.

use std::{num::NonZeroU32, rc::Rc};

use demi_agent_tools::{
    ChosenWorkflow, ConversationProfiles, EFFORT_OFF, HostResolver, NewProfileDraft, Profile,
    ProfileEdit, SubagentSettings, Unavailable, WorkflowListing,
};
use demi_shared_types::{ModelSelection, NodeId, THINKING_OFF, WorkflowChoice};
use futures_util::future::join_all;
use schemars::JsonSchema;
use serde::Serialize;

use super::Tree;
use crate::{Node, server::AgentServer};

/// Why a spawn fails while the user has subagents off.
const SUBAGENTS_OFF: &str = "subagents are turned off in settings; do this work in this session, or ask the user to turn subagents on in Settings > Subagents.";

/// Why a spawn fails in a conversation whose Workflow menu says No
/// Subagents.
const OFF_HERE: &str = "subagents are turned off for this conversation; do this work in this session, or ask the user to choose Default or a workflow in the composer's Workflow menu.";

/// What a spawn makes its child of: the profile's settings, or the
/// parent's for the inherit profile.
pub(super) struct Setup {
    pub(super) model: ModelSelection,
    /// The instructions the profile adds to the child's system prompt; none
    /// when it adds none, and for the inherit profile, whose child copies
    /// its parent's.
    pub(super) instructions: Option<String>,
    pub(super) can_spawn: bool,
    /// The limits the settings set at that moment, which the spawn checks
    /// next ([`Limits`]).
    pub(super) limits: Limits,
}

/// The user's two limits on a conversation's subagents (`subagents.md`
/// § Limits), each none for Unlimited.
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
    /// False while the user has subagents off with Default chosen, with No
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
struct ListedProfile {
    name: String,
    description: String,
    /// The message a spawn with the profile fails with now, which says
    /// what is missing; null while it is available.
    unavailable: Option<String>,
}

impl<H: HostResolver> Tree<H> {
    /// What a spawn by `owner` with the profile `name`, none for the inherit
    /// profile, makes its child of. The checks run in their order against
    /// the user's current settings and the conversation's choice, and the
    /// first that fails ends the spawn before anything is created:
    /// subagents are on, the name is known, the profile is enabled, and its
    /// model settings make a selection. A workflow's profiles replace the
    /// user's, whatever the user's switch is. The limits the spawn checks
    /// next come with the answer.
    pub(super) async fn spawn_setup(
        &self,
        server: &AgentServer<H>,
        owner: &Node<H>,
        name: Option<&str>,
    ) -> Result<Setup, String> {
        let settings = read(server).await?;
        let limits = Limits::of(&settings);
        let inherit = || Setup {
            model: owner.session().model(),
            instructions: None,
            can_spawn: true,
            limits,
        };
        match self.conversation_profiles(server).await? {
            ConversationProfiles::Off => Err(OFF_HERE.to_owned()),
            ConversationProfiles::Unavailable { id, reason } => Err(format!(
                "this conversation's workflow \"{id}\" is unavailable: {reason}; do this work in this session, or ask the user to choose another in the composer's Workflow menu."
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
                let model = match &child.model {
                    None => owner.session().model(),
                    Some(Ok(selection)) => selection.clone(),
                    Some(Err(missing)) => {
                        return Err(format!("profile \"{name}\" is unavailable: {missing}"));
                    }
                };
                Ok(Setup {
                    model,
                    instructions: child.instructions.clone(),
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
                let model = match &profile.model {
                    Some(model) => server
                        .deps
                        .providers
                        .profile_selection(model)
                        .await
                        .map_err(|missing| unavailable(name, &missing))?,
                    None => owner.session().model(),
                };
                Ok(Setup {
                    model,
                    instructions: profile.instructions.clone(),
                    can_spawn: profile.can_spawn,
                    limits,
                })
            }
        }
    }

    /// `demi agent workflows list`: the workflows of the conversation's
    /// working directory and its choice, read now.
    pub(crate) async fn workflows(&self) -> Result<WorkflowListing, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        server.deps.subagents.workflows(self.root.id()).await
    }

    /// The text of the file `path`, relative to `caller`'s working
    /// directory, which `demi agent workflows check` parses.
    pub(crate) async fn workflow_file(&self, caller: &NodeId, path: &str) -> Result<String, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let directory = self.directory_of(caller).await?;
        server
            .deps
            .subagents
            .read_workflow_file(self.root.id(), &directory, path)
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
    /// conversation's choice now: the chosen workflow's profiles in its
    /// order; or, with Default and subagents on, each enabled profile of the
    /// user's in name order; an unavailable one with what is missing.
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

/// `demi agent profiles show --json` and the answer of a profile change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ProfileAnswer {
    profile: ShownProfile,
}

/// Every field of one profile, enabled or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ShownProfile {
    name: String,
    description: String,
    /// Null for the parent's model.
    model: Option<ShownModel>,
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
    /// The entry's name as Settings shows it; null once the entry is gone.
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
        let settings = read(&server).await?;
        let profile = settings
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| no_profile(name, &settings))?;
        Ok(answer(&server, profile).await)
    }

    /// `demi agent profiles create`: the profile as stored.
    pub(crate) async fn create_profile(
        &self,
        draft: NewProfileDraft,
    ) -> Result<ProfileAnswer, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let created = server.deps.subagents.create_profile(draft).await?;
        Ok(answer(&server, &created).await)
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
        let profile = &self.profile;
        let yes = |value: bool| if value { "yes" } else { "no" };
        let model = match &profile.model {
            None => "parent's".to_owned(),
            Some(model) => {
                let provider = model.provider_name.as_deref().unwrap_or(&model.provider);
                let effort = model
                    .effort
                    .as_ref()
                    .map(|effort| format!(", effort {effort}"))
                    .unwrap_or_default();
                let tier = model.tier.as_deref().unwrap_or("default");
                format!("{} from {provider}{effort}, tier {tier}", model.model)
            }
        };
        let mut text = format!(
            "name          {}\ndescription   {}\nmodel         {model}\nspawning      {}\nenabled       {}\n",
            profile.name,
            profile.description,
            yes(profile.spawning),
            yes(profile.enabled),
        );
        if let Some(unavailable) = &profile.unavailable {
            text.push_str(&format!("unavailable   {unavailable}\n"));
        }
        match &profile.instructions {
            None => text.push_str("instructions  none\n"),
            Some(instructions) => {
                text.push_str("instructions\n");
                text.push_str(instructions);
                if !instructions.ends_with('\n') {
                    text.push('\n');
                }
            }
        }
        text
    }
}

/// `profile` as `show` gives it, checked now.
async fn answer<H: HostResolver>(server: &AgentServer<H>, profile: &Profile) -> ProfileAnswer {
    let missing = match &profile.model {
        Some(model) => server.deps.providers.profile_selection(model).await.err(),
        None => None,
    };
    ProfileAnswer {
        profile: ShownProfile {
            name: profile.name.clone(),
            description: profile.description.clone(),
            model: profile.model.as_ref().map(|model| ShownModel {
                provider: model.provider_id.clone(),
                provider_name: model.provider_name.clone(),
                model: model.model_id.clone(),
                effort: model.thinking_effort.as_ref().map(|effort| {
                    if effort == THINKING_OFF {
                        EFFORT_OFF.to_owned()
                    } else {
                        effort.clone()
                    }
                }),
                tier: model.service_tier_id.clone(),
            }),
            instructions: profile.instructions.clone(),
            spawning: profile.can_spawn,
            enabled: profile.enabled,
            unavailable: missing.map(|missing| unavailable(&profile.name, &missing)),
        },
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
    /// The text `demi agent profiles list` prints: a profile per line, its name
    /// padded to one column, and under an unavailable one why a spawn with
    /// it fails.
    pub(crate) fn render(&self) -> String {
        let off = match &self.choice {
            WorkflowChoice::Default if !self.enabled => Some(
                "Subagents are turned off in settings; demi agent spawn fails until the user turns them on.".to_owned(),
            ),
            WorkflowChoice::None => Some(
                "Subagents are turned off for this conversation; demi agent spawn fails until the user chooses Default or a workflow in the composer's Workflow menu.".to_owned(),
            ),
            WorkflowChoice::Workflow { id } if !self.enabled => Some(format!(
                "This conversation's workflow \"{id}\" is unavailable: {}; demi agent spawn fails until the user chooses another in the composer's Workflow menu.",
                self.unavailable.as_deref().unwrap_or_default()
            )),
            WorkflowChoice::Default | WorkflowChoice::Workflow { .. } => None,
        };
        if let Some(off) = off {
            return format!("{off}\n");
        }
        let heading = match &self.workflow {
            None if self.profiles.is_empty() => {
                return "The user has no enabled subagent profile; demi agent spawn without --profile inherits the parent.\n".to_owned();
            }
            None => String::new(),
            Some(name) if self.profiles.is_empty() => format!(
                "Workflow {name}: no profiles; demi agent spawn without --profile inherits the parent.\n"
            ),
            Some(name) => format!("Workflow {name}:\n"),
        };
        let width = self
            .profiles
            .iter()
            .map(|profile| profile.name.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        let mut text = heading;
        for profile in &self.profiles {
            text.push_str(&format!(
                "{:width$}{}\n",
                profile.name, profile.description
            ));
            if let Some(unavailable) = &profile.unavailable {
                text.push_str(&format!("{:width$}{unavailable}\n", ""));
            }
        }
        text
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
    ListedProfile {
        name: profile.name.clone(),
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
