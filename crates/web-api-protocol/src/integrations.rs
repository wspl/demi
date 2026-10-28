//! The user's integrations (`crates/backend-user-shard/src/integrations.rs`):
//! what Settings › Integrations reads and sends.

use demi_shared_types::{IntegrationCapabilities, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::IntegrationId;
use crate::text::Trimmed;

/// An integration as Settings › Integrations shows it: never its token, which
/// only its creation and its replacement answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationDto {
    pub id: IntegrationId,
    pub name: String,
    pub capabilities: IntegrationCapabilities,
    pub created_at: Timestamp,
    /// How many Clouds it holds.
    pub clouds: u32,
    /// How many conversations it holds.
    pub conversations: u32,
}

/// The user's integrations, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationList {
    pub integrations: Vec<IntegrationDto>,
}

/// An integration with its token, as Add Integration and Replace Token
/// answer: the one time a page sees the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationWithToken {
    pub integration: IntegrationDto,
    pub token: String,
}

/// An integration as a change answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationAnswer {
    pub integration: IntegrationDto,
}

/// A change of an integration's settings; a field left out stays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrationPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed", length(min = 1))]
    #[garde(inner(length(chars, min = 1)))]
    pub name: Option<Trimmed>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "IntegrationCapabilities")]
    #[garde(skip)]
    pub capabilities: Option<IntegrationCapabilities>,
}
