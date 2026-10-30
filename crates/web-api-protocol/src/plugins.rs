//! A user's plugins: the backend's plugins as the settings page lists them, the
//! switch that turns one or one of its features on or off for the user, and a
//! plugin's state for one conversation.

use std::collections::BTreeMap;

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
    /// Whether the user has each of its features on, by the feature's id
    /// (`crates/plugin-host/src/features.rs`).
    pub features: BTreeMap<String, bool>,
    /// The command packages its commands, user streams and page methods
    /// bind that the backend's catalog serves, such as `demi.browser`.
    pub packages: Vec<String>,
}

/// `PUT /plugins/:plugin { enabled }` and `PUT
/// /plugins/:plugin/features/:feature { enabled }`.
#[derive(Debug, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PluginSwitch {
    #[garde(skip)]
    pub enabled: bool,
}

/// `GET /conversations/:id/plugins/:plugin/state`: the plugin's state for the
/// conversation, valid against the plugin's declared schema, and the revision
/// it was read at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct PluginStateAnswer {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    #[garde(skip)]
    pub state: serde_json::Value,
}

/// The most reply blocks one read of their facts names: a page asks for the
/// blocks the replies it shows name, and a bound keeps a request from asking
/// a plugin without end.
pub const REPLY_BLOCKS_PER_READ: usize = 256;

/// A plugin's reply block a reply names, by its tag and its ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedReplyBlock {
    #[garde(length(min = 1, max = 64))]
    pub tag: String,
    #[garde(length(min = 1, max = 128))]
    pub id: String,
}

/// `POST /reply-blocks { blocks }`: the facts of reply blocks the page holds
/// replies naming but no facts of, such as those of a copy it kept, which a
/// transcript's pages and frames bring otherwise.
#[derive(Debug, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ReplyBlocksRead {
    #[garde(length(min = 1, max = REPLY_BLOCKS_PER_READ), dive)]
    pub blocks: Vec<NamedReplyBlock>,
}

/// What `POST /reply-blocks` answers: each block's facts, null for one the
/// user does not have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplyBlocksAnswer {
    pub reply_blocks: Vec<demi_conversation_lane_protocol::ReplyBlockFacts>,
}
