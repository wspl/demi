//! Devices: the list, pairing, and a Host's log (`web-api.md` § Workspaces,
//! devices, and attached hosts, § Device log).

use demi_core::{Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::DeviceId;

/// How a device came to be: `user` for one its user paired, `managed` for
/// the user's Cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    User,
    Managed,
}

serde_plain::derive_display_from_serialize!(DeviceKind);
serde_plain::derive_fromstr_from_deserialize!(DeviceKind);

/// A device as the browser sees it. `online` says whether its runner is
/// connected now; `home` is the home directory it reported when it last
/// connected, null until then (the backend keeps it in memory only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDto {
    pub id: DeviceId,
    pub kind: DeviceKind,
    pub name: String,
    pub platform: String,
    pub claimed_at: Timestamp,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub last_seen_at: Option<Timestamp>,
    pub online: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub home: Option<String>,
}

/// `GET /devices`: the caller's paired devices, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Devices {
    pub devices: Vec<DeviceDto>,
}

/// `POST /devices/claim`: the pairing code a waiting runner printed, spelled
/// in any case, with any spaces and dashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    #[garde(length(utf16, min = 1))]
    pub code: String,
}

/// `{ device }`: the answer of a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClaimedDevice {
    pub device: DeviceDto,
}

/// One line of a Host's log (`runner.md` § Host log).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLogLine {
    pub at: Timestamp,
    pub source: String,
    /// The conversation the work belonged to, when it belonged to one.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub conversation_id: Option<String>,
    pub text: String,
}

/// `GET /devices/:id/log`: lines oldest first, and the cursor the next read
/// continues from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceLog {
    pub lines: Vec<DeviceLogLine>,
    pub next: u64,
}

/// `?since=&limit=&source=` of the device log. Queries are the backend's
/// alone and are not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct DeviceLogQuery {
    /// The `next` of an earlier answer; without it the answer ends at the
    /// newest line.
    #[serde(default)]
    pub since: Option<u64>,
    #[serde(default)]
    pub limit: LogLimit,
    #[serde(default)]
    pub source: Option<LogSource>,
}

/// How many lines a device log read returns: 1 to what one runner read
/// returns, 200 when omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "u64")]
pub struct LogLimit(u64);

impl LogLimit {
    pub const DEFAULT: LogLimit = LogLimit(200);

    pub fn get(self) -> u64 {
        self.0
    }
}

impl Default for LogLimit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u64> for LogLimit {
    type Error = String;

    fn try_from(limit: u64) -> Result<Self, String> {
        let most = demi_runner_protocol::wire::LOG_READ_LINES as u64;
        if (1..=most).contains(&limit) {
            Ok(Self(limit))
        } else {
            Err(format!("limit must be 1 to {most}"))
        }
    }
}

/// The one source a device log read keeps, such as `runner`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct LogSource(String);

impl LogSource {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LogSource {
    type Error = &'static str;

    fn try_from(source: String) -> Result<Self, &'static str> {
        if source.is_empty() {
            Err("source must not be empty")
        } else {
            Ok(Self(source))
        }
    }
}
