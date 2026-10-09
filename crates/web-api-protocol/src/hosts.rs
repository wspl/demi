//! A conversation's attached hosts as the web app lists them
//! (`web-api.md` § Workspaces, devices, and attached hosts); the user
//! detaches one, and the conversation's agents attach them.

use demi_shared_types::{Nullable, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::devices::DeviceState;
use crate::ids::DeviceId;

/// A device attached to a conversation.
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
    /// Whether the device's runner serves it, as the device list says.
    pub state: DeviceState,
    pub attached_at: Timestamp,
}

/// `{ hosts }`: the conversation's attached hosts, first attached first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttachedHosts {
    pub hosts: Vec<AttachedHost>,
}
