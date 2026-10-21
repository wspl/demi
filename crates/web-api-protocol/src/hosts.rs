//! A conversation's attached hosts as the web app lists them; the user detaches
//! one, and the conversation's agents attach them.

use demi_shared_types::{MAX_SAFE_INTEGER, Nullable, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::DeviceId;

/// A device attached to a conversation. Whether its runner serves it is the
/// device list's, which the page holds already.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachedHost {
    pub device_id: DeviceId,
    /// What the model and the user call the host; unique within the
    /// conversation.
    pub name: String,
    /// The directory its commands start in, fixed when it was attached; null
    /// for a device attached by name, whose commands start in its home.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub cwd: Option<String>,
    pub attached_at: Timestamp,
}

/// The conversation's attached hosts, first attached first, under the
/// revision an attach, a detach or a rename raises, 0 before the first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationHosts {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    pub hosts: Vec<AttachedHost>,
}
