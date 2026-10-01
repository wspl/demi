//! The product state and the messages of the page's synchronization channel
//! (`web-api.md` § Page synchronization).

use demi_shared_types::Nullable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auth::UserDto;
use crate::cloud::CloudStatus;
use crate::conversations::ConversationSummary;
use crate::devices::DeviceDto;
use crate::exposes::ExposeDto;
use crate::ids::ConversationId;
use crate::providers::ProviderState;
use crate::settings::{InstanceMode, Preferences};
use crate::workspaces::WorkspaceDto;

/// The product state: the signed-in user, the instance mode, the user's
/// preferences, the provider entries the user infers with, the user's
/// workspaces in their order, the user's devices, the paired ones and the
/// Cloud, the user's live exposes, soonest expiry first, with the domain of
/// their hostnames, null when the instance has none and exposes are off,
/// the backend's public URL, the summaries of the user's conversations, the
/// active ones first, then the archived, and the Cloud's status. The backend
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
    pub exposes: Vec<ExposeDto>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub expose_domain: Option<String>,
    /// The URL runners connect to (`DEMI_BACKEND_PUBLIC_URL`), whose origin
    /// serves the installers: the page's install command names it, since
    /// the page's own origin may be another server's, as in development.
    pub public_url: String,
    pub conversations: Vec<ConversationSummary>,
    pub cloud: CloudStatus,
}

/// A message of the page's synchronization channel, `WS /sync`: the whole
/// product state first, then the current value of each part of it that
/// changed, each part's in the order of its changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    /// The whole product state, first on every connection; it replaces
    /// whatever the page held.
    Snapshot { state: Box<ProductState> },
    /// A conversation's summary, as the conversation lists carry it.
    Conversation { conversation: Box<ConversationSummary> },
    /// The id of every conversation, in the product state's order.
    ConversationOrder { ids: Vec<ConversationId> },
    Preferences { preferences: Preferences },
    User { user: UserDto },
    /// The user's workspaces, in the user's order.
    Workspaces { workspaces: Vec<WorkspaceDto> },
    /// The paired devices and the Cloud's.
    Devices { devices: Vec<DeviceDto> },
    /// The live exposes, soonest expiry first.
    Exposes { exposes: Vec<ExposeDto> },
    /// The entries the user infers with, each with what its provider says.
    Providers { providers: Vec<ProviderState> },
    Cloud { cloud: CloudStatus },
    /// Nothing else was sent for 30 seconds.
    Heartbeat,
}
