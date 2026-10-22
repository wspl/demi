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
use demi_conversation_socket_protocol::{Held, Version};

use crate::ids::ConversationId;
use crate::passkeys::PasskeyDto;
use crate::plugins::PluginEntry;
use crate::providers::{ListedLogin, ModelCatalog, ProviderState};
use crate::sessions::SessionDto;
use crate::settings::{InstanceMode, Preferences};
use crate::subagents::SubagentSettings;
use crate::users::ListedUser;
use crate::workspaces::WorkspaceDto;

/// The product state `GET /state` answers: the signed-in user, the instance
/// mode, the user's preferences, the provider entries the user infers with and
/// the catalog of their models, the user's workspaces in their order, the
/// user's devices, the paired ones and the Cloud, the backend's public URL, the
/// summaries of the user's conversations, the active ones first, then the
/// archived, the Cloud's status, the backend's plugins with whether the user has
/// each on, and the state of each plugin the user has on that gives one, by its
/// id. The lists only a settings pane shows are read when it opens.
/// The backend reads it without waking a Cloud or running inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProductState {
    pub user: UserDto,
    pub mode: InstanceMode,
    pub preferences: Preferences,
    pub providers: Vec<ProviderState>,
    /// The models of the entries the user infers with, as the model menu
    /// lists them, so the model a conversation names is known before it
    /// opens.
    pub models: ModelCatalog,
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
    /// The runs that notified and whose conversation the user has not
    /// opened since, of any age: the sidebar's count.
    pub automations_unread: u32,
    /// The relying party's id of the backend's passkeys, as `GET /setup`
    /// gives it.
    pub passkey_rp_id: Option<String>,
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
    /// revisions it counts in memory, such as a conversation's list of
    /// permission requests, compare only within one run: a page holding
    /// one of another run reads again.
    pub run: String,
    /// The version of the product state this read is current as of: the page
    /// follows the state with it on the synchronization channel
    /// (`SyncFollowed::State`).
    pub version: Version,
}

/// A value of the product a page follows on its synchronization channel: the
/// product state, or a list only a settings pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncFollowed {
    /// The product state `GET /state` answers, held at its version
    /// (`Held::Version`).
    State,
    Instructions,
    Subagents,
    Credentials,
    PortForwards,
    ExposedPorts,
    Automations,
    Passkeys,
    Sessions,
    Users,
}

/// A value a page follows on its synchronization channel, and the version of
/// it the page holds: the state's version, or a list's entity tag, empty for
/// a list whose route sends none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncFollow {
    pub value: SyncFollowed,
    pub holds: Held,
}

/// A message a page sends on its synchronization channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum PageMessage {
    /// The channel's first message: the values the page shows, each with the
    /// version it holds. Sent once.
    Open { follows: Vec<SyncFollow> },
    /// A value the page reads later, such as a settings list its pane opened.
    Follow(SyncFollow),
    /// The value's view closed.
    Unfollow { value: SyncFollowed },
    /// The conversation the page shows while it is focused and visible, or
    /// null, each time that changes; no push notification of it reaches the
    /// session's browser while it does.
    Showing {
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "demi_shared_types::Nullable<ConversationId>")]
        showing: Option<ConversationId>,
    },
}

/// A message of the page's synchronization channel, `WS /sync`: the current
/// value of each part of the product state, and of each settings list, that
/// changed after the version the page follows it from, each part's in the
/// order of its changes. A page reads the state by `GET /state` first, then
/// opens the channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    /// The product state is current as of `version`: the parts sent before
    /// it are the changes up to it. A page follows the state from the latest
    /// one it applied.
    Synced { version: Version },
    /// The value's version the page holds cannot be brought current: the
    /// follow ended; the page reads the value again and follows it again.
    Stale { value: SyncFollowed },
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
    /// The catalog of the entries' models changed: an entry, the
    /// preferences Automatic reads or a source's list did.
    Models {
        models: ModelCatalog,
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
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// The user saved their personal instructions.
    Instructions {
        instructions: String,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// A credential was saved, used or removed: the user's
    /// credentials, without their values.
    Credentials {
        credentials: Vec<CredentialDto>,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// A port forward was added, changed or removed, or its Listen On
    /// runner announced a new state of it: the user's forwards.
    PortForwards {
        #[serde(rename = "portForwards")]
        port_forwards: Vec<crate::port_forwards::PortForwardDto>,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// An exposed port was added, changed or removed: the user's exposed
    /// ports.
    ExposedPorts {
        #[serde(rename = "exposedPorts")]
        exposed_ports: Vec<crate::exposed_ports::ExposedPortDto>,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// An automation was created, changed or deleted, or one of its runs
    /// started, ended or was read, and the page follows the automations: the
    /// user's automations.
    Automations {
        automations: Vec<AutomationDto>,
        /// As the product state's `automations_unread`.
        #[serde(rename = "automationsUnread")]
        automations_unread: u32,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// A passkey was added, renamed, removed or signed in with: the user's
    /// passkeys, as `GET /auth/passkeys` lists them.
    Passkeys {
        passkeys: Vec<PasskeyDto>,
        #[serde(rename = "userHandle")]
        user_handle: String,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// A session of the account started, ended or was signed out, or its
    /// last activity was written: the account's sessions, as
    /// `GET /auth/sessions` lists them for the channel's session.
    Sessions {
        sessions: Vec<SessionDto>,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// An automation's run started, ended or was read, and the page follows
    /// the state alone: the sidebar's count.
    AutomationsUnread {
        #[serde(rename = "automationsUnread")]
        automations_unread: u32,
    },
    /// An account was added, changed, reset, disabled or enabled, signed in,
    /// or chose its password: every account, as `GET /users` lists them. Only
    /// the channels of the master and admins carry it.
    Users {
        users: Vec<ListedUser>,
        /// The entity tag of the list's route's answer; empty when it has none.
        etag: String,
    },
    /// Nothing else was sent for 30 seconds.
    Heartbeat,
    /// The answer to the page's `{ ping: id }`, sent at once.
    Pong { id: u32 },
}
