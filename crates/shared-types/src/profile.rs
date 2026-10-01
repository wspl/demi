//! A named subagent configuration (`subagents.md` § Profiles), as data: what
//! a plugin declares and what the agent server assembles a child with.

use serde::{Deserialize, Serialize};

use crate::ModelSelection;

/// Every field overrides what a child would inherit from its parent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    /// The `--profile` value.
    pub name: String,
    /// What the profile is for, listed in the spawn command's help.
    pub description: String,
    /// Replaces the instructions in the child's system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// The command paths, such as `["demi", "file"]`, the child keeps of its
    /// parent's commands; every other command is left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<Vec<String>>>,
    /// Whether the profile's children may spawn children of their own.
    pub can_spawn_subagents: bool,
    /// A model used instead of the parent's, on a fork of the parent's
    /// provider runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelection>,
}

impl Profile {
    /// The `--profile` value that selects no profile: the child inherits its
    /// parent's configuration. No profile may be named so.
    pub const INHERIT: &'static str = "default";
}
