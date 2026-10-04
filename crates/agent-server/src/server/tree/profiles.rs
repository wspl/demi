//! The user's subagent settings as a tree reads them (`subagents.md`
//! § Profiles): the checks a spawn makes against the settings as they are at
//! that moment, and `demi agent profiles`. Nothing here is kept between two
//! reads, so a change in settings reaches the next spawn of every tree.

use std::rc::Rc;

use demi_agent_tools::{HostResolver, Profile, SubagentSettings, Unavailable};
use demi_shared_types::ModelSelection;
use futures_util::future::join_all;
use schemars::JsonSchema;
use serde::Serialize;

use super::Tree;
use crate::{Node, server::AgentServer};

/// Why a spawn fails while the user has subagents off.
const SUBAGENTS_OFF: &str = "subagents are turned off in settings; do this work in this session, or ask the user to turn subagents on in Settings > Subagent.";

/// What a spawn makes its child of: the profile's settings, or the
/// parent's for the inherit profile.
pub(super) struct Setup {
    pub(super) model: ModelSelection,
    /// The instructions that replace the parent's; none for the parent's.
    pub(super) instructions: Option<String>,
    pub(super) can_spawn: bool,
}

/// `demi agent profiles --json`: the Subagent switch, and each enabled
/// profile while it is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ProfileListing {
    enabled: bool,
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
    /// the user's current settings, and the first that fails ends the spawn
    /// before anything is created: subagents are on, the name is known, the
    /// profile is enabled, and its model settings make a selection.
    pub(super) async fn spawn_setup(
        &self,
        server: &AgentServer<H>,
        owner: &Node<H>,
        name: Option<&str>,
    ) -> Result<Setup, String> {
        let settings = read(server).await?;
        if !settings.enabled {
            return Err(SUBAGENTS_OFF.to_owned());
        }
        let Some(name) = name else {
            return Ok(Setup {
                model: owner.session().model(),
                instructions: None,
                can_spawn: true,
            });
        };
        let profile = settings
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| unknown(name, &settings))?;
        if !profile.enabled {
            return Err(format!(
                "profile \"{name}\" is disabled in settings; omit --profile to inherit the parent, or ask the user to enable it in Settings > Subagent."
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
        })
    }

    /// `demi agent profiles`, from the user's settings now: while subagents
    /// are on, each enabled profile in name order, an unavailable one with
    /// what is missing; while they are off, none.
    pub(crate) async fn profiles(&self) -> Result<ProfileListing, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let settings = read(&server).await?;
        if !settings.enabled {
            return Ok(ProfileListing {
                enabled: false,
                profiles: Vec::new(),
            });
        }
        let enabled = settings.profiles.iter().filter(|profile| profile.enabled);
        let profiles = join_all(enabled.map(|profile| listed(&server, profile))).await;
        Ok(ProfileListing {
            enabled: true,
            profiles,
        })
    }
}

impl ProfileListing {
    /// The text `demi agent profiles` prints: a profile per line, its name
    /// padded to one column, and under an unavailable one why a spawn with
    /// it fails.
    pub(crate) fn render(&self) -> String {
        if !self.enabled {
            return "Subagents are turned off in settings; demi agent spawn fails until the user turns them on.\n".to_owned();
        }
        if self.profiles.is_empty() {
            return "The user has no enabled subagent profile; demi agent spawn without --profile inherits the parent.\n".to_owned();
        }
        let width = self
            .profiles
            .iter()
            .map(|profile| profile.name.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        let mut text = String::new();
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

/// An enabled profile as `demi agent profiles` lists it, checked now.
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
        "unknown profile \"{name}\"; the user's profiles are {}. Run `demi agent profiles` for when to use each, or omit --profile to inherit the parent.",
        names.join(", ")
    )
}

/// Why a spawn with the unavailable profile `name` fails.
fn unavailable(name: &str, missing: &Unavailable) -> String {
    format!("profile \"{name}\" is unavailable: {missing}")
}
