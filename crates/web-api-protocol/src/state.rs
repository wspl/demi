//! The product state and the messages of the page's synchronization channel.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::UserDto;
use crate::automations::AutomationDto;
use crate::cloud::CloudStatus;
use crate::conversations::ConversationSummary;
use crate::credentials::CredentialDto;
use crate::devices::{DeviceDto, InstallCommands, StunUrl};
use crate::ids::ConversationId;
use crate::passkeys::{PasskeyDto, PasskeyList};
use crate::plugins::PluginEntry;
use crate::providers::{ListedLogin, ProviderState};
use crate::sessions::SessionDto;
use crate::settings::{InstanceMode, Preferences};
use crate::subagents::SubagentSettings;
use crate::users::ListedUser;
use crate::workspaces::WorkspaceDto;

/// The product state: the signed-in user, the instance mode, the user's
/// preferences, the provider entries the user infers with, the user's
/// workspaces in their order, the user's devices, the paired ones and the
/// Cloud, the backend's public URL, the summaries of the user's
/// conversations, the active ones first, then the archived, the Cloud's
/// status, the user's Subagents switch, limits and profiles, the user's
/// credentials without their values, the backend's
/// plugins with whether the user has each on, and the state of each plugin
/// the user has on that gives one, by its id. The backend reads it for each
/// channel, without waking a Cloud or running inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProductState {
    pub user: UserDto,
    pub mode: InstanceMode,
    pub preferences: Preferences,
    pub providers: Vec<ProviderState>,
    pub workspaces: Vec<WorkspaceDto>,
    pub devices: Vec<DeviceDto>,
    /// The URL runners connect to (`DEMI_BACKEND_PUBLIC_URL`).
    pub public_url: String,
    /// The commands that install the runner on each system, which fetch the
    /// installer from the origin of `public_url`, not from the page's own,
    /// which may be another server's, as in development.
    pub install_commands: InstallCommands,
    /// The STUN servers the page's peers ask for the address the internet
    /// sees the browser at (`DEMI_STUN_URLS`, `crates/runner-direct/src/lib.rs`); none keeps direct channels to one network.
    pub stun_urls: Vec<StunUrl>,
    /// The runner release paired devices follow, the one the backend's
    /// `runners/` names; none for a backend without runner releases, as in
    /// development, whose runners are development builds
    /// (`packages/web-ui/src/devices/direct.ts`).
    pub runner_release: Option<String>,
    pub conversations: Vec<ConversationSummary>,
    pub cloud: CloudStatus,
    /// The user's sign-ins that run or ended within ten minutes, in the
    /// order they started, as the `logins` message carries them.
    pub logins: Vec<ListedLogin>,
    pub subagents: SubagentSettings,
    /// The user's personal instructions; empty when they have none.
    pub instructions: String,
    /// The user's credentials in name order, without their values.
    pub credentials: Vec<CredentialDto>,
    /// The user's port forwards, oldest first, each with what its Listen On
    /// runner last announced of it.
    pub port_forwards: Vec<crate::port_forwards::PortForwardDto>,
    /// The user's exposed ports, oldest first.
    pub exposed_ports: Vec<crate::exposed_ports::ExposedPortDto>,
    /// The user's automations, newest first, each with its latest runs.
    pub automations: Vec<AutomationDto>,
    /// The runs that notified and whose conversation the user has not
    /// opened since, of any age: the sidebar's count.
    pub automations_unread: u32,
    /// The user's passkeys and WebAuthn user handle.
    pub passkeys: PasskeyList,
    /// The relying party's id of the backend's passkeys, as `GET /setup`
    /// gives it.
    pub passkey_rp_id: Option<String>,
    /// The account's signed-in browsers, as `GET /auth/sessions` lists them
    /// for the channel's session.
    pub sessions: Vec<SessionDto>,
    /// Every account, as `GET /users` lists them, for the master and
    /// admins; none for a user, whose channel never carries the part.
    pub users: Option<Vec<ListedUser>>,
    /// The backend's plugins, in their order of registration.
    pub plugins: Vec<PluginEntry>,
    /// The state of each plugin the user has on that gives one, valid
    /// against the schema its page package's types are generated from.
    pub plugin_states: BTreeMap<String, Value>,
    /// The build of the web app the backend serves, from its `build.json`,
    /// or none when it serves none, as in development: a page of another
    /// build is out of date.
    pub web_build: Option<String>,
    /// Whether the backend can send account mail, which an email change
    /// needs for its code; without it the page offers no email change.
    pub mail: bool,
    /// This run of the backend, an id it chooses when it starts. The
    /// revisions it counts in memory, such as a summary's
    /// `permissionsRevision`, compare only within one run: a page holding
    /// one of another run reads again.
    pub run: String,
}

/// The one message a page sends on its synchronization channel: the
/// conversation it shows while it is focused and visible, or null, each time
/// that changes; no push notification of it reaches the session's browser
/// while it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageShowing {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "demi_shared_types::Nullable<ConversationId>")]
    pub showing: Option<ConversationId>,
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
    /// The conversation was deleted: the page drops its summary, and a page
    /// that shows it goes to a new conversation.
    ConversationDeleted {
        id: ConversationId,
    },
    /// The id of every conversation, in the product state's order.
    ConversationOrder {
        ids: Vec<ConversationId>,
    },
    Preferences {
        preferences: Preferences,
    },
    /// The account changed: its nickname, its email or its role, or its
    /// owner chose the password. The first message of the channel of an
    /// account still to choose its password, which carries nothing else until
    /// it is chosen; the product state follows then.
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
    /// A sign-in of the user's started, moved to another stage, showed its
    /// link or code, took or refused a code, completed, failed, was
    /// cancelled or was dropped: the user's sign-ins that run or ended
    /// within ten minutes.
    Logins {
        logins: Vec<ListedLogin>,
    },
    /// The user turned subagents on or off, or created, changed, enabled,
    /// disabled or deleted a profile.
    Subagents {
        subagents: SubagentSettings,
    },
    /// The user saved their personal instructions.
    Instructions {
        instructions: String,
    },
    /// A credential was saved, used or removed: the user's
    /// credentials, without their values.
    Credentials {
        credentials: Vec<CredentialDto>,
    },
    /// A port forward was added, changed or removed, or its Listen On
    /// runner announced a new state of it: the user's forwards.
    PortForwards {
        #[serde(rename = "portForwards")]
        port_forwards: Vec<crate::port_forwards::PortForwardDto>,
    },
    /// An exposed port was added, changed or removed: the user's exposed
    /// ports.
    ExposedPorts {
        #[serde(rename = "exposedPorts")]
        exposed_ports: Vec<crate::exposed_ports::ExposedPortDto>,
    },
    /// An automation was created, changed or deleted, or one of its runs
    /// started, ended or was read: the user's automations, as the product
    /// state carries them.
    Automations {
        automations: Vec<AutomationDto>,
        /// As the product state's `automations_unread`.
        #[serde(rename = "automationsUnread")]
        automations_unread: u32,
    },
    /// A passkey was added, renamed, removed or signed in with: the user's
    /// passkeys, as `GET /auth/passkeys` lists them.
    Passkeys {
        passkeys: Vec<PasskeyDto>,
        #[serde(rename = "userHandle")]
        user_handle: String,
    },
    /// A session of the account started, ended or was signed out, or its
    /// last activity was written: the account's sessions, as
    /// `GET /auth/sessions` lists them for the channel's session.
    Sessions {
        sessions: Vec<SessionDto>,
    },
    /// An account was added, changed, reset, disabled or enabled, signed in,
    /// or chose its password: every account, as `GET /users` lists them. Only
    /// the channels of the master and admins carry it.
    Users {
        users: Vec<ListedUser>,
    },
    /// Nothing else was sent for 30 seconds.
    Heartbeat,
    /// The answer to the page's `{ ping: id }`, sent at once.
    Pong { id: u32 },
}
