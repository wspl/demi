//! A user's subagent profiles (`web-api.md` § Subagents): the rules a
//! profile's name and texts follow, which a body passes before it is
//! written, and how a patch merges into a stored profile. The model a body
//! names is checked against its entry's catalog by the user's shard.

use demi_shared_types::is_blank;
use demi_web_api_protocol::subagents::{NewProfile, ProfilePatch, SubagentProfile};

/// The name no profile takes: omitting `--profile` selects the inherit
/// profile, and naming it would read as that one.
pub const RESERVED_NAME: &str = "default";
/// The most characters a profile's name has.
pub const NAME_MAX: usize = 40;
/// The most characters a profile's description has.
pub const DESCRIPTION_MAX: usize = 500;
/// The most characters of the instructions a profile replaces a child's
/// with.
pub const INSTRUCTIONS_MAX: usize = 65_536;

/// Why a profile's name or text is refused. Each names the field, as a
/// refused body does.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileTextError {
    #[error(
        "name: must be 1 to {NAME_MAX} lowercase letters, digits and hyphens, starting with a letter"
    )]
    Name,
    #[error("name: \"{RESERVED_NAME}\" is reserved for inheriting the parent")]
    ReservedName,
    #[error("description: must be one line of 1 to {DESCRIPTION_MAX} characters")]
    Description,
    #[error("instructions: must be 1 to {INSTRUCTIONS_MAX} characters")]
    Instructions,
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

/// `profile` with `patch` applied: each field the patch names replaces the
/// stored one, `model` and `instructions` whole, and the others stay.
pub fn merge(profile: SubagentProfile, patch: ProfilePatch) -> SubagentProfile {
    SubagentProfile {
        id: profile.id,
        name: patch.name.unwrap_or(profile.name),
        description: patch.description.unwrap_or(profile.description),
        model: patch.model.unwrap_or(profile.model),
        instructions: patch.instructions.unwrap_or(profile.instructions),
        can_spawn: patch.can_spawn.unwrap_or(profile.can_spawn),
        enabled: patch.enabled.unwrap_or(profile.enabled),
    }
}

/// 1 to 40 lowercase letters, digits and hyphens, starting with a letter,
/// and not the reserved name.
fn check_name(name: &str) -> Result<(), ProfileTextError> {
    let starts_with_letter = name.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    let allowed = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !starts_with_letter || !allowed || name.chars().count() > NAME_MAX {
        return Err(ProfileTextError::Name);
    }
    if name == RESERVED_NAME {
        return Err(ProfileTextError::ReservedName);
    }
    Ok(())
}

/// One line that is not blank, of at most 500 characters.
fn check_description(description: &str) -> Result<(), ProfileTextError> {
    let one_line = !description.contains(['\n', '\r']);
    if is_blank(description) || !one_line || description.chars().count() > DESCRIPTION_MAX {
        return Err(ProfileTextError::Description);
    }
    Ok(())
}

/// Text that is not blank, of at most 65,536 characters.
fn check_instructions(instructions: &str) -> Result<(), ProfileTextError> {
    if is_blank(instructions) || instructions.chars().count() > INSTRUCTIONS_MAX {
        return Err(ProfileTextError::Instructions);
    }
    Ok(())
}
