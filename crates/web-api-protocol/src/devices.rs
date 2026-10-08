//! Devices: the list, pairing, and a Host's log (`web-api.md` § Workspaces,
//! devices, and attached hosts, § Device log).

use demi_runner_protocol::direct::CANDIDATE_CHARS;
/// The STUN servers' URLs, which the product state names to the page.
pub use demi_runner_protocol::direct::{NotAStunUrl, StunUrl};
use demi_runner_protocol::wire::{HostArtifact, OperatingSystem, RunnerPlatform};
use demi_shared_types::{MAX_SAFE_INTEGER, Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::{DeviceId, WorkspaceId};
use crate::text::Trimmed;

/// The most characters a device's name has.
pub const DEVICE_NAME_MAX: usize = 64;

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

/// Whether a device's runner serves it: `online` while its runner is
/// connected, `updating` while it replaces itself with the backend's runner
/// release (`runner.md` § Runner updates), `offline` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Online,
    Updating,
    Offline,
}

/// A device as the web app sees it. `platform` is the one its runner
/// reported; `state` says whether its runner serves it now; `home` is
/// the home directory it reported when it last connected, null until then
/// (the backend keeps it in memory only); `installed` is what its runner
/// last reported its artifact cache holds, kept while it is offline
/// (`native-runtime.md` § Installed artifacts); `os` and `runner_version`
/// are the operating system and the runner release its runner last
/// reported, null before its runner first connected; `direct` is whether
/// pages may reach it over a direct channel (`direct-channel.md` § Choosing
/// the path); `start_command` is what a person types in a terminal on a
/// paired device to start its runner again, null for the Cloud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDto {
    pub id: DeviceId,
    pub kind: DeviceKind,
    pub name: String,
    pub platform: RunnerPlatform,
    pub claimed_at: Timestamp,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub last_seen_at: Option<Timestamp>,
    pub state: DeviceState,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub home: Option<String>,
    pub installed: Vec<HostArtifact>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<OperatingSystem>")]
    pub os: Option<OperatingSystem>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub runner_version: Option<String>,
    pub direct: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub start_command: Option<String>,
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
    #[garde(length(chars, min = 1))]
    pub code: String,
}

/// `PATCH /devices/:id`: a paired device's new name, whether pages may
/// reach it directly, or both; each applies when present.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeDevice {
    #[serde(default)]
    #[garde(inner(length(chars, min = 1, max = DEVICE_NAME_MAX)))]
    pub name: Option<Trimmed>,
    #[serde(default)]
    #[garde(skip)]
    pub direct: Option<bool>,
}

/// `{ device }`: the answer of a claim and of a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceAnswer {
    pub device: DeviceDto,
}

/// `{ removed }`: the answer of a revocation, the ids of the workspaces
/// that went with the device (`web-api.md` § Workspaces, devices, and
/// attached hosts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RevokedDevice {
    pub removed: Vec<WorkspaceId>,
}

/// One line of a Host's log (`runner.md` § Host log).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLogLine {
    pub at: Timestamp,
    pub source: String,
    /// The conversation the work belonged to, when it belonged to one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    pub conversation_id: Option<String>,
    pub text: String,
}

/// `GET /devices/:id/log`: lines oldest first, and the cursor the next read
/// continues from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceLog {
    pub lines: Vec<DeviceLogLine>,
    #[garde(range(max = MAX_SAFE_INTEGER))]
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

/// A message the page sends on a device's direct channel signaling socket,
/// `WS /devices/:deviceId/direct` (`web-api.md` § Direct channel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectRequest {
    /// The page's offer for a new peer, which replaces the socket's peer.
    Offer {
        #[garde(length(min = 1, max = MAX_OFFER_BYTES))]
        sdp: String,
    },
    /// A candidate the browser found after the offer, the `candidate:` line
    /// as it gives it; the runner checks it too.
    Candidate {
        #[garde(length(min = 1, max = CANDIDATE_CHARS))]
        candidate: String,
    },
}

/// The most bytes of an offer: a data channel's offer is a few kilobytes.
pub const MAX_OFFER_BYTES: usize = 64 * 1024;

/// A message the backend sends on a direct channel signaling socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DirectMessage {
    /// The runner's answer to the page's last offer.
    Answer { sdp: String },
    /// A candidate the runner found after its answer, such as the address a
    /// STUN server saw it at; it may come before the answer.
    Candidate { candidate: String },
    /// The runner did not answer the page's last offer.
    Unanswered { code: Unanswered },
    /// Nothing else was sent for 30 seconds: the socket is quiet, not dead.
    Heartbeat,
    /// The runner closed the socket's peer, since the user turned a plugin
    /// on or off: the page makes a new offer at once, whose introduction
    /// carries the user's streams as they are now.
    Closed,
}

/// Why an offer has no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Unanswered {
    /// The runner keeps as many peers as it may.
    Busy,
    /// The runner could not answer the offer.
    InvalidOffer,
    /// The runner did not answer within 10 seconds.
    Timeout,
}
