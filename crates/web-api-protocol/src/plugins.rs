//! A user's plugins (`web-api.md` § A user's plugins): the backend's
//! plugins as the settings page lists them, the switch that turns one on
//! or off for the user, and a plugin's state for one conversation.

use demi_shared_types::MAX_SAFE_INTEGER;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A plugin of the backend, in its order of registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    /// What it does, in one sentence.
    pub description: String,
    /// Whether the user has it on.
    pub enabled: bool,
    /// The command packages its commands, user streams and page methods
    /// bind that the backend's catalog serves, such as `demi.browser`.
    pub packages: Vec<String>,
}

/// `PUT /plugins/:plugin { enabled }`.
#[derive(Debug, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PluginSwitch {
    #[garde(skip)]
    pub enabled: bool,
}

/// `GET /conversations/:id/plugins/:plugin/state` (`web-api.md`
/// § Conversation state of plugins): the plugin's state for the
/// conversation, valid against the plugin's declared schema, and the
/// revision it was read at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct PluginStateAnswer {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    #[garde(skip)]
    pub state: serde_json::Value,
}
