//! The user's subagent settings as a tree reads them (`subagents.md`
//! § Profiles, § Limits, § Managing profiles): the checks a spawn makes
//! against the settings as they are at that moment, `demi agent profiles`
//! and `demi agent models`. Nothing here is kept between two reads, so a
//! change in settings reaches the next spawn of every tree.

use std::{num::NonZeroU32, rc::Rc};

use demi_agent_tools::{
    EFFORT_OFF, EntryKind, HostResolver, MenuEntry, NewProfileDraft, Profile, ProfileEdit,
    SubagentSettings, Unavailable,
};
use demi_shared_types::{ModelSelection, NodeId, THINKING_OFF};
use futures_util::future::join_all;
use schemars::JsonSchema;
use serde::Serialize;

use super::Tree;
use crate::{Node, server::AgentServer};

/// Why a spawn fails while the user has subagents off.
const SUBAGENTS_OFF: &str = "subagents are turned off in settings; do this work in this session, or ask the user to turn subagents on in Settings > Subagents.";

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

/// `demi agent profiles list --json`: the Subagents switch, and each enabled
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
    /// profile is enabled, and its model settings make a selection. The
    /// limits the spawn checks next come with the answer.
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
        let limits = Limits::of(&settings);
        let Some(name) = name else {
            return Ok(Setup {
                model: owner.session().model(),
                instructions: None,
                can_spawn: true,
                limits,
            });
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

    /// The limits the user's settings set now, which a resume checks.
    pub(super) async fn limits(&self, server: &AgentServer<H>) -> Result<Limits, String> {
        Ok(Limits::of(&read(server).await?))
    }

    /// `demi agent profiles list`, from the user's settings now: while subagents
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

/// `demi agent models --json`: what the composer's model menu offers the
/// user now (`subagents.md` § Managing profiles).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct ModelListing {
    providers: Vec<ListedEntry>,
}

/// A provider entry of the menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
struct ListedEntry {
    id: String,
    /// The entry's name as Settings shows it.
    name: String,
    kind: ListedKind,
    models: Vec<ListedModel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ListedKind {
    Subscription,
    ApiKey,
}

/// A model of the menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedModel {
    id: String,
    name: String,
    /// The context window in tokens; null when the catalog does not say.
    window: Option<u32>,
    efforts: Vec<String>,
    /// The effort a profile that names none takes: the first one listed.
    default_effort: Option<String>,
    /// Whether `--effort off` turns its thinking off.
    can_turn_off: bool,
    tiers: Vec<String>,
    images: bool,
    /// Whether the calling agent runs on it.
    current: bool,
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
    /// `demi agent models`: the menu's catalog now, the model `caller` runs
    /// on marked.
    pub(crate) async fn models(&self, caller: &NodeId) -> Result<ModelListing, String> {
        let server = self.server.upgrade().ok_or("the agent server is gone")?;
        let entries = server.deps.subagents.models().await?;
        let current = self.node(caller).map(|node| node.session().model());
        Ok(ModelListing::of(entries, current.as_ref()))
    }

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

impl ModelListing {
    fn of(entries: Vec<MenuEntry>, current: Option<&ModelSelection>) -> Self {
        let providers = entries
            .into_iter()
            .map(|entry| {
                let models = entry
                    .models
                    .into_iter()
                    .map(|model| ListedModel {
                        current: current.is_some_and(|current| {
                            current.provider_id == entry.id && current.model.id == model.id
                        }),
                        default_effort: model.efforts.first().cloned(),
                        id: model.id,
                        name: model.name,
                        window: model.window,
                        efforts: model.efforts,
                        can_turn_off: model.can_turn_off,
                        tiers: model.tiers,
                        images: model.images,
                    })
                    .collect();
                ListedEntry {
                    id: entry.id,
                    name: entry.name,
                    kind: match entry.kind {
                        EntryKind::Subscription => ListedKind::Subscription,
                        EntryKind::ApiKey => ListedKind::ApiKey,
                    },
                    models,
                }
            })
            .collect();
        Self { providers }
    }

    /// The text `demi agent models` prints: each entry's name and kind, then
    /// a row per model in columns, the calling agent's marked with `*`, as
    /// is each default effort, and the mark's key last.
    pub(crate) fn render(&self) -> String {
        if self.providers.is_empty() {
            return "The model menu offers no model: the user has no provider entry that can run, or turned every one off in Settings > Providers.\n".to_owned();
        }
        let rows: Vec<(&ListedEntry, Vec<[String; 6]>)> = self
            .providers
            .iter()
            .map(|entry| {
                let rows = entry
                    .models
                    .iter()
                    .map(|model| {
                        let mut efforts: Vec<String> = model
                            .efforts
                            .iter()
                            .enumerate()
                            .map(|(index, effort)| {
                                if index == 0 {
                                    format!("{effort}*")
                                } else {
                                    effort.clone()
                                }
                            })
                            .collect();
                        if model.can_turn_off {
                            efforts.push(EFFORT_OFF.to_owned());
                        }
                        [
                            format!("{} {}", if model.current { '*' } else { ' ' }, model.id),
                            model.name.clone(),
                            model.window.map_or_else(|| "-".to_owned(), window),
                            efforts.join(" "),
                            model.tiers.join(" "),
                            if model.images { "images" } else { "-" }.to_owned(),
                        ]
                    })
                    .collect();
                (entry, rows)
            })
            .collect();
        let mut widths = [0; 6];
        for row in rows.iter().flat_map(|(_, rows)| rows) {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let mut text = String::new();
        for (entry, rows) in rows {
            let kind = match entry.kind {
                ListedKind::Subscription => "subscription",
                ListedKind::ApiKey => "API key",
            };
            text.push_str(&format!("{} · {kind}\n", entry.name));
            for row in rows {
                // A column no model fills, such as tiers when none has one,
                // is left out.
                let line: Vec<String> = row
                    .iter()
                    .zip(widths)
                    .filter(|(_, width)| *width > 0)
                    .map(|(cell, width)| format!("{cell:width$}"))
                    .collect();
                text.push_str(line.join("  ").trim_end());
                text.push('\n');
            }
        }
        text.push_str("* the model this agent runs on; an effort with * is the model's default\n");
        text
    }
}

/// A context window as a menu shows it: `1M`, `128K`, `1.2M`.
fn window(tokens: u32) -> String {
    let scaled = |value: f64, unit: &str| {
        let text = format!("{value:.1}");
        format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
    };
    match tokens {
        1_000_000.. => scaled(f64::from(tokens) / 1_000_000.0, "M"),
        1_000.. => scaled(f64::from(tokens) / 1_000.0, "K"),
        _ => tokens.to_string(),
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

/// Why a spawn with the unavailable profile `name` fails.
fn unavailable(name: &str, missing: &Unavailable) -> String {
    format!("profile \"{name}\" is unavailable: {missing}")
}
