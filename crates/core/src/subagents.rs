//! The user's subagent settings: the Subagents switch, the limits and the
//! profiles.
//!
//! They are written for the settings page and for `demi agent profiles`
//! alike, after the same checks:
//!
//! - A profile's name and texts follow their rules, and a patch merges into
//!   the stored profile.
//! - Its model is checked against its entry's catalog as a conversation's model
//!   settings are, and stored with its effort explicit.
//!
//! Each change reaches the user's pages as the `subagents` part, and the next
//! spawn of every conversation reads it, since a spawn reads the settings each
//! time.

use std::fmt;

use demi_core_host_access::transition::ChangeRefusal;
use demi_core_store::StorageError;
use demi_core_store::subagents::ProfileRefusal as Stored;
use demi_core_sync::Part;
use demi_shared_types::{context_word, is_blank, offers_context};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::ProfileId;
use demi_web_api_protocol::subagents::{NewProfile, ProfilePatch, SubagentProfile, SubagentsPatch};

use crate::Core;

/// The name no profile takes: omitting `--profile` selects the inherit
/// profile, and naming it would read as that one.
pub const RESERVED_NAME: &str = "default";

/// Why a profile's name or text is refused. Each names the field, as a
/// refused body does.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileTextError {
    #[error("name: must be lowercase letters, digits and hyphens, starting with a letter")]
    Name,
    #[error("name: \"{RESERVED_NAME}\" is reserved for inheriting the parent")]
    ReservedName,
    #[error("description: must be one line that is not blank")]
    Description,
    #[error("instructions: must not be blank")]
    Instructions,
}

/// Why a profile's Context is refused
/// (`crates/agent-server/src/server/tree/profiles.rs`).
///
/// The Context needs a model of the profile's own, and one the model's window
/// offers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextRefusal {
    #[error(
        "context: the profile inherits the parent's model, which follows the user's context limit; give the profile a model of its own first"
    )]
    NoModel,
    #[error("{0}")]
    NotOffered(NotOffered),
}

/// A Context of `tokens` that the model `model`, of a window of `window`,
/// does not offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotOffered {
    pub model: String,
    pub tokens: u32,
    pub window: u32,
}

impl fmt::Display for NotOffered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { model, tokens, window } = self;
        if *window == 0 {
            return write!(
                f,
                "context: model \"{model}\" has no known window, so it offers no Context of its own; use default"
            );
        }
        write!(
            f,
            "context: model \"{model}\" has a window of {}, under {}; use a size up to it, full, or default for the user's limit",
            context_word(*window),
            context_word(*tokens),
        )
    }
}

/// Checks a profile's Context `tokens`, none for the user's limit, against
/// its model `model` with that model's window, none for the parent's model.
pub fn check_context(model: Option<(&str, u32)>, tokens: Option<u32>) -> Result<(), ContextRefusal> {
    let Some(tokens) = tokens else {
        return Ok(());
    };
    let (model, window) = model.ok_or(ContextRefusal::NoModel)?;
    if offers_context(window, tokens) {
        Ok(())
    } else {
        Err(ContextRefusal::NotOffered(NotOffered {
            model: model.to_owned(),
            tokens,
            window,
        }))
    }
}

/// Checks a new profile's name and texts.
pub fn check_new(profile: &NewProfile) -> Result<(), ProfileTextError> {
    check_name(&profile.name)?;
    check_description(&profile.description)?;
    profile
        .instructions
        .as_deref()
        .map_or(Ok(()), check_instructions)
}

/// Checks the name and texts a patch names.
pub fn check_patch(patch: &ProfilePatch) -> Result<(), ProfileTextError> {
    patch.name.as_deref().map_or(Ok(()), check_name)?;
    patch
        .description
        .as_deref()
        .map_or(Ok(()), check_description)?;
    patch
        .instructions
        .as_ref()
        .and_then(Option::as_deref)
        .map_or(Ok(()), check_instructions)
}

/// `profile` with `patch` applied.
///
/// Each field the patch names replaces the stored one, `model` and
/// `instructions` whole, and the others stay. A patch that changes the model
/// and names no Context keeps the stored one while `window`, the new model's,
/// offers it. The parent's model, whose window is none, keeps none.
pub fn merge(profile: SubagentProfile, patch: ProfilePatch, window: Option<u32>) -> SubagentProfile {
    let context_limit = match (patch.context_limit, &patch.model) {
        (Some(chosen), _) => chosen,
        (None, Some(_)) => profile
            .context_limit
            .filter(|tokens| window.is_some_and(|window| offers_context(window, *tokens))),
        (None, None) => profile.context_limit,
    };
    SubagentProfile {
        id: profile.id,
        name: patch.name.unwrap_or(profile.name),
        description: patch.description.unwrap_or(profile.description),
        model: patch.model.unwrap_or(profile.model),
        context_limit,
        instructions: patch.instructions.unwrap_or(profile.instructions),
        can_spawn: patch.can_spawn.unwrap_or(profile.can_spawn),
        enabled: patch.enabled.unwrap_or(profile.enabled),
    }
}

/// Lowercase letters, digits and hyphens, starting with a letter,
/// and not the reserved name.
fn check_name(name: &str) -> Result<(), ProfileTextError> {
    let starts_with_letter = name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    let allowed = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !starts_with_letter || !allowed {
        return Err(ProfileTextError::Name);
    }
    if name == RESERVED_NAME {
        return Err(ProfileTextError::ReservedName);
    }
    Ok(())
}

/// One line that is not blank.
fn check_description(description: &str) -> Result<(), ProfileTextError> {
    let one_line = !description.contains(['\n', '\r']);
    if is_blank(description) || !one_line {
        return Err(ProfileTextError::Description);
    }
    Ok(())
}

/// Text that is not blank.
fn check_instructions(instructions: &str) -> Result<(), ProfileTextError> {
    if is_blank(instructions) {
        return Err(ProfileTextError::Instructions);
    }
    Ok(())
}

/// Why a profile was not written.
#[derive(Debug, thiserror::Error)]
pub enum ProfileRefusal {
    /// A name or a text breaks its rule.
    #[error(transparent)]
    Text(#[from] ProfileTextError),
    /// The Context needs a model of the profile's own, or one its model
    /// offers.
    #[error(transparent)]
    Context(#[from] ContextRefusal),
    #[error(transparent)]
    Stored(#[from] Stored),
    /// The model is outside the user's scope, unlisted, or does not offer
    /// the effort or the tier.
    #[error(transparent)]
    Model(ChangeRefusal),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl Core {
    /// Sets what `patch` names of the user's Subagents switch and limits.
    pub async fn patch_subagents(&self, patch: SubagentsPatch) -> Result<(), StorageError> {
        self.store().set_subagents(patch)
            .await?;
        self.mark_subagents();
        Ok(())
    }

    /// Creates the user's profile `profile`, enabled or not.
    pub async fn create_profile(
        &self,
        mut profile: NewProfile,
        enabled: bool,
    ) -> Result<SubagentProfile, ProfileRefusal> {
        check_new(&profile)?;
        let checked = match &profile.model {
            Some(model) => Some(self.profile_model(model).await?),
            None => None,
        };
        check_context(
            checked.as_ref().map(|(model, window)| (model.model_id.as_str(), *window)),
            profile.context_limit,
        )?;
        profile.model = checked.map(|(model, _)| model);
        let created = self
            .store().create_profile(profile, enabled)
            .await??;
        self.mark_subagents();
        Ok(created)
    }

    /// Applies `patch` to the user's profile `id`. Turning it on or off
    /// checks nothing else, so an unavailable profile can be disabled and
    /// enabled as it is.
    pub async fn patch_profile(
        &self,
        id: ProfileId,
        mut patch: ProfilePatch,
    ) -> Result<SubagentProfile, ProfileRefusal> {
        check_patch(&patch)?;
        // The model the profile has after the patch, with its window: the
        // patch's, or the stored one's when the patch names a Context alone.
        let model = match &patch.model {
            Some(Some(model)) => Some(self.profile_model(model).await?),
            Some(None) => None,
            None => match patch.context_limit {
                Some(Some(_)) => match self.stored_profile(&id).await?.model {
                    Some(stored) => Some(self.profile_model(&stored).await?),
                    None => None,
                },
                _ => None,
            },
        };
        if let Some(chosen) = patch.context_limit {
            check_context(
                model.as_ref().map(|(model, window)| (model.model_id.as_str(), *window)),
                chosen,
            )?;
        }
        let window = model.as_ref().map(|(_, window)| *window);
        if let Some(Some(_)) = patch.model {
            patch.model = Some(model.map(|(model, _)| model));
        }
        let changed = self
            .store().patch_profile(id, move |stored| {
                merge(stored, patch, window)
            })
            .await??;
        self.mark_subagents();
        Ok(changed)
    }

    /// Deletes the user's profile `id`.
    pub async fn delete_profile(&self, id: ProfileId) -> Result<(), ProfileRefusal> {
        let deleted = self
            .store().delete_profile(id)
            .await?;
        if !deleted {
            return Err(Stored::NotFound.into());
        }
        self.mark_subagents();
        Ok(())
    }

    /// `model` as a profile stores it, checked against the entry's catalog,
    /// with the model's first effort when it names none, and the model's
    /// context window, which a profile's Context is checked against.
    pub(crate) async fn profile_model(
        &self,
        model: &ModelSettings,
    ) -> Result<(ModelSettings, u32), ProfileRefusal> {
        let selection = self
            .chosen_selection(model)
            .await
            .map_err(ProfileRefusal::Model)?;
        let settings = ModelSettings {
            provider_id: model.provider_id.clone(),
            model_id: selection.model.id.clone(),
            thinking_effort: selection.thinking_effort().map(str::to_owned),
            service_tier_id: selection.service_tier_id.clone(),
        };
        Ok((settings, selection.model.context_window))
    }

    /// The user's profile `id` as stored.
    async fn stored_profile(&self, id: &ProfileId) -> Result<SubagentProfile, ProfileRefusal> {
        let settings = self
            .store().subagent_settings()
            .await?;
        settings
            .profiles
            .into_iter()
            .find(|profile| profile.id == *id)
            .ok_or(Stored::NotFound.into())
    }

    fn mark_subagents(&self) {
        self.services().sync.mark(self.user(), Part::Subagents);
    }
}
