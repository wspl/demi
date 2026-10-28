//! What an integration's agents may do
//! (`crates/backend-user-shard/src/integrations.rs`), which its creator
//! switches in its editor and the integration through its API.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The capabilities an integration turns on or off for its agents. Each is
/// a group of commands, or for subagents the `demi agent` group's spawning;
/// what an integration never has, such as credentials, is no field here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct IntegrationCapabilities {
    /// Its agents spawn subagents.
    pub subagents: bool,
    /// The browser plugin's commands.
    pub browser: bool,
    /// Web search and fetching.
    pub web: bool,
    /// The skills of the integration's own skill sources.
    pub skills: bool,
    /// Images, videos and audio its agents make.
    pub media: bool,
    /// iOS simulators and Android emulators.
    pub mobile: bool,
    /// Schedules that wake a conversation.
    pub schedules: bool,
    /// Files its agents attach to their replies.
    pub attachments: bool,
}

impl Default for IntegrationCapabilities {
    /// Every capability on: a new integration's agents do what a user's do,
    /// short of what no integration has.
    fn default() -> Self {
        Self {
            subagents: true,
            browser: true,
            web: true,
            skills: true,
            media: true,
            mobile: true,
            schedules: true,
            attachments: true,
        }
    }
}
