//! The product snapshot the page loads and revalidates (`web-api.md` §
//! Sidebar mutations, read state and page synchronization).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auth::UserDto;
use crate::conversations::ConversationSummary;
use crate::devices::DeviceDto;
use crate::providers::ProviderState;
use crate::settings::{InstanceMode, Preferences};

/// `GET /state`: the signed-in user, the instance mode, the user's
/// preferences, the provider entries the user infers with, the user's
/// devices, the paired ones and the Cloud, and the summaries of the user's
/// conversations, the active ones first, then the archived. The backend
/// assembles it on each request, without waking a Cloud or running
/// inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProductState {
    pub user: UserDto,
    pub mode: InstanceMode,
    pub preferences: Preferences,
    pub providers: Vec<ProviderState>,
    pub devices: Vec<DeviceDto>,
    pub conversations: Vec<ConversationSummary>,
}
