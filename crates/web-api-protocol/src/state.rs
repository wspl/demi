//! The product state and the messages of the page's synchronization channel
//! (`web-api.md` § Page synchronization).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::UserDto;
use crate::cloud::CloudStatus;
use crate::conversations::ConversationSummary;
use crate::devices::DeviceDto;
use crate::ids::ConversationId;
use crate::plugins::PluginEntry;
use crate::providers::ProviderState;
use crate::settings::{InstanceMode, Preferences};
use crate::subagents::SubagentSettings;
use crate::workspaces::WorkspaceDto;

/// The product state: the signed-in user, the instance mode, the user's
/// preferences, the provider entries the user infers with, the user's
/// workspaces in their order, the user's devices, the paired ones and the
/// Cloud, the backend's public URL, the summaries of the user's
/// conversations, the active ones first, then the archived, the Cloud's
/// status, the user's Subagent switch and profiles, the backend's plugins with whether the user has each on, and the
/// state of each plugin the user has on that gives one, by its id. The backend
/// reads it for each channel, without waking a Cloud or running inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProductState {
    pub user: UserDto,
    pub mode: InstanceMode,
    pub preferences: Preferences,
    pub providers: Vec<ProviderState>,
    pub workspaces: Vec<WorkspaceDto>,
    pub devices: Vec<DeviceDto>,
    /// The URL runners connect to (`DEMI_BACKEND_PUBLIC_URL`), whose origin
    /// serves the installers: the page's install command names it, since
    /// the page's own origin may be another server's, as in development.
    pub public_url: String,
    pub conversations: Vec<ConversationSummary>,
    pub cloud: CloudStatus,
    pub subagents: SubagentSettings,
    /// The backend's plugins, in their order of registration.
    pub plugins: Vec<PluginEntry>,
    /// The state of each plugin the user has on that gives one, valid
    /// against the schema its page package's types are generated from.
    pub plugin_states: BTreeMap<String, Value>,
    /// The build of the web app the backend serves, from its `build.json`,
    /// or none when it serves none, as in development: a page of another
    /// build is out of date (`web-application.md` § A page of another
    /// build).
    pub web_build: Option<String>,
    /// Whether the backend can send account mail, which an email change
    /// needs for its code (`web-api.md` § Account API); without it the page
    /// offers no email change.
    pub mail: bool,
    /// This run of the backend, an id it chooses when it starts. The
    /// revisions it counts in memory, such as a summary's
    /// `permissionsRevision`, compare only within one run: a page holding
    /// one of another run reads again (`web-api.md` § Revisions counted in
    /// memory).
    pub run: String,
}

/// A message of the page's synchronization channel, `WS /sync`: the whole
/// product state first, then the current value of each part of it that
/// changed, each part's in the order of its changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    /// The whole product state, first on every connection; it replaces
    /// whatever the page held.
    Snapshot {
        state: Box<ProductState>,
    },
    /// A conversation's summary, as the conversation lists carry it.
    Conversation {
        conversation: Box<ConversationSummary>,
    },
    /// The id of every conversation, in the product state's order.
    ConversationOrder {
        ids: Vec<ConversationId>,
    },
    Preferences {
        preferences: Preferences,
    },
    User {
        user: UserDto,
    },
    /// The user's workspaces, in the user's order.
    Workspaces {
        workspaces: Vec<WorkspaceDto>,
    },
    /// The paired devices and the Cloud's.
    Devices {
        devices: Vec<DeviceDto>,
    },
    /// The user turned a plugin on or off.
    Plugins {
        plugins: Vec<PluginEntry>,
    },
    /// A plugin's state for the user's pages changed.
    Plugin {
        plugin: String,
        state: Value,
    },
    /// The entries the user infers with, each with what its provider says.
    Providers {
        providers: Vec<ProviderState>,
    },
    Cloud {
        cloud: CloudStatus,
    },
    /// The user turned subagents on or off, or created, changed, enabled,
    /// disabled or deleted a profile.
    Subagents {
        subagents: SubagentSettings,
    },
    /// Nothing else was sent for 30 seconds.
    Heartbeat,
}
