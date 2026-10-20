//! The caller's subagent settings (`web-api-settings.md` § Subagents): the Subagents
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
    /// Maximum subagents: how many subagents may be live at once across
    /// all the user's conversations (`crates/agent-server/src/server/tree/profiles.rs`).
    pub max_live: SubagentLimit,
    /// Maximum depth: how many levels of subagents a tree may have below
    /// its root.
    pub max_depth: SubagentLimit,
    /// The caller's profiles, in name order.
    pub profiles: Vec<SubagentProfile>,
}

/// A limit on a conversation's subagents (`crates/agent-server/src/server/tree/profiles.rs`): the
/// whole number from 1 the user last set, which stays while the limit is
/// lifted, so that choosing Limited again brings it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentLimit {
    #[garde(skip)]
    pub limit: NonZeroU32,
    /// Whether the limit is lifted: Unlimited.
    #[garde(skip)]
    pub unlimited: bool,
}

impl SubagentLimit {
    /// The bound a spawn checks against: none while the limit is lifted.
    pub fn bound(self) -> Option<NonZeroU32> {
        (!self.unlimited).then_some(self.limit)
    }
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
    /// The tokens of the profile's own Context, its model's full window or
    /// a limit the window offers, which its children use in place of the
    /// user's limit (`crates/agent-server/src/server/tree/profiles.rs`); null for the user's limit,
    /// and always null without a model of its own.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    pub context_limit: Option<u32>,
    /// The text added to a child's system prompt after the product's; null
    /// for none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub instructions: Option<String>,
    /// Whether the profile's children may spawn children of their own.
    pub can_spawn: bool,
    /// Whether agents may use the profile.
    pub enabled: bool,
}

/// `PUT /subagents`: the fields it names change, and the others stay. A
/// limit is a whole value, its number and whether it is lifted.
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
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "SubagentLimit")]
    #[garde(dive)]
    pub max_live: Option<SubagentLimit>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "SubagentLimit")]
    #[garde(dive)]
    pub max_depth: Option<SubagentLimit>,
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
    /// Null for the user's limit; otherwise the model's full window or a
    /// limit it offers, in tokens, which needs a model of the profile's own.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(skip)]
    pub context_limit: Option<u32>,
    /// Null for no instructions added to the product's.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub instructions: Option<String>,
    #[garde(skip)]
    pub can_spawn: bool,
}

/// `PATCH /subagents/profiles/:id`: the fields it names change, and the
/// others stay. `model` and `instructions` are whole values: a null model
/// is the parent's, and null instructions add none. A null `contextLimit`
/// is the user's limit; a model changed without one keeps the profile's
/// while the new model offers it, and the parent's model takes none.
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
    #[schemars(with = "Option<u32>")]
    #[garde(skip)]
    pub context_limit: Option<Option<u32>>,
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
