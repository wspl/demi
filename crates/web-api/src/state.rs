//! The product snapshot the page loads and revalidates (`web-api.md` §
//! Sidebar mutations, read state and page synchronization).

use demi_core::Nullable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auth::UserDto;
use crate::cloud::CloudStatus;
use crate::conversations::ConversationSummary;
use crate::devices::DeviceDto;
use crate::exposes::ExposeDto;
use crate::providers::ProviderState;
use crate::settings::{InstanceMode, Preferences};
use crate::workspaces::WorkspaceDto;

/// `GET /state`: the signed-in user, the instance mode, the user's
/// preferences, the provider entries the user infers with, the user's
/// workspaces in their order, the user's devices, the paired ones and the
/// Cloud, the user's live exposes, soonest expiry first, with the domain of
/// their hostnames, null when the instance has none and exposes are off,
/// the backend's public URL, the summaries of the user's conversations, the
/// active ones first, then the archived, and the Cloud's status. The backend
/// assembles it on each request, without waking a Cloud or running inference.
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
