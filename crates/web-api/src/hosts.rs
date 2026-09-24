//! A conversation's attached hosts as the browser lists and changes them
//! (`web-api.md` § Workspaces, devices, and attached hosts).

use demi_core::{Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::DeviceId;
use crate::text::Trimmed;

/// The most characters of an attached host's name.
pub const HOST_NAME_MAX: usize = 64;

/// A device attached to a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachedHost {
    pub device_id: DeviceId,
    /// What the model and the user call the host; unique within the
    /// conversation.
    pub name: String,
    /// Where the last shell on the host ended, where the next one starts;
    /// null until one ran.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub cwd: Option<String>,
    /// Whether the device's runner is connected.
    pub online: bool,
    pub attached_at: Timestamp,
}

/// `{ hosts }`: the conversation's attached hosts, first attached first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttachedHosts {
    pub hosts: Vec<AttachedHost>,
}

/// `POST /conversations/:id/hosts { deviceId }`: a device of the user's to
/// attach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachHost {
    #[garde(skip)]
    pub device_id: DeviceId,
}

/// `PATCH /conversations/:id/hosts/:deviceId { name }`: 1 to 64 characters
/// after trimming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct RenameHost {
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1, max = HOST_NAME_MAX))]
    pub name: Trimmed,
}
