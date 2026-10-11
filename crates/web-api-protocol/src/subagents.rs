//! The caller's subagent settings (`web-api.md` § Subagents): the Subagents
//! switch, the limits and the subagent profiles, with the bodies that change them. The
//! name and text rules are the accounts service's, which checks a body
//! before it is written.

use std::num::NonZeroU32;

use demi_shared_types::Nullable;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::conversations::ModelSettings;
use crate::ids::ProfileId;

/// `subagents` of the product state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSettings {
    /// The Subagents switch: true for a user who never turned it off.
    pub enabled: bool,
    /// Maximum subagents: how many subagents of one conversation may be
    /// live at once (`subagents.md` § Limits); null for Unlimited.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<NonZeroU32>")]
    pub max_live: Option<NonZeroU32>,
    /// Maximum depth: how many levels of subagents a tree may have below
    /// its root; null for Unlimited.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<NonZeroU32>")]
    pub max_depth: Option<NonZeroU32>,
    /// The caller's profiles, in name order.
    pub profiles: Vec<SubagentProfile>,
}

/// One of the caller's subagent profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProfile {
    pub id: ProfileId,
    /// The `--profile` value.
    pub name: String,
    /// When the agent should use the profile.
    pub description: String,
    /// The model settings a child infers with; null for its parent's model.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ModelSettings>")]
    pub model: Option<ModelSettings>,
    /// The text that replaces a child's instructions; null for its parent's.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub instructions: Option<String>,
    /// Whether the profile's children may spawn children of their own.
    pub can_spawn: bool,
    /// Whether agents may use the profile.
    pub enabled: bool,
}

/// `PUT /subagents`: the fields it names change, and the others stay. A
/// limit is a whole number from 1, or null for Unlimited.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentsPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub enabled: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<NonZeroU32>")]
    #[garde(skip)]
    pub max_live: Option<Option<NonZeroU32>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<NonZeroU32>")]
    #[garde(skip)]
    pub max_depth: Option<Option<NonZeroU32>>,
}

/// `POST /subagents/profiles`: a new profile, which starts enabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewProfile {
    #[garde(skip)]
    pub name: String,
    #[garde(skip)]
    pub description: String,
    /// Null for the parent's model. An effort of null takes the first effort
    /// the model lists.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ModelSettings>")]
    #[garde(dive)]
    pub model: Option<ModelSettings>,
    /// Null for the parent's instructions.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub instructions: Option<String>,
    #[garde(skip)]
    pub can_spawn: bool,
}

/// `PATCH /subagents/profiles/:id`: the fields it names change, and the
/// others stay. `model` and `instructions` are whole values, and null
/// returns either to the parent's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfilePatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<ModelSettings>")]
    #[garde(dive)]
    pub model: Option<Option<ModelSettings>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<String>")]
    #[garde(skip)]
    pub instructions: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub can_spawn: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub enabled: Option<bool>,
}

/// `{ profile }`: the answer of a create and of a patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileAnswer {
    pub profile: SubagentProfile,
}
