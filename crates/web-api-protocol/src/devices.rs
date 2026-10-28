//! Devices: the list, pairing, and a Host's log.

use demi_runner_protocol::direct::CANDIDATE_CHARS;
/// The STUN servers' URLs, which the product state names to the page.
pub use demi_runner_protocol::direct::{NotAStunUrl, StunUrl};
use demi_runner_protocol::wire::{AutostartUnavailable, HostArtifact, HostDownload, OperatingSystem, RunnerPlatform};
use demi_shared_types::{Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, ProjectId};
use crate::text::Trimmed;

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

/// How pages reach a paired device (`packages/web/src/direct/index.ts`): `automatic`, over a direct channel while its path is not worse
/// than the relay's; `direct`, over one whenever it connects, Prefer
/// P2P; `server`, never over one, Relay Only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRoute {
    Automatic,
    Direct,
    Server,
}

serde_plain::derive_display_from_serialize!(DeviceRoute);
serde_plain::derive_fromstr_from_deserialize!(DeviceRoute);

/// The logo of a device's mark: its system's, Apple for macOS, the Windows logo,
/// for Linux its distribution's, or Tux for any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceLogo {
    Apple,
    Windows,
    Ubuntu,
    Debian,
    Fedora,
    Arch,
    Centos,
    Redhat,
    Mint,
    Tux,
}

impl DeviceLogo {
    /// The logo of a device on `platform` whose runner last reported `os`:
    /// before its runner first connected, a Linux device's distribution is
    /// unknown, and it shows Tux.
    pub fn of(platform: RunnerPlatform, os: Option<&OperatingSystem>) -> Self {
        match platform {
            RunnerPlatform::Darwin => Self::Apple,
            RunnerPlatform::Win32 => Self::Windows,
            RunnerPlatform::Linux => {
                match os.and_then(|os| os.distribution.as_deref()) {
                    Some("ubuntu") => Self::Ubuntu,
                    Some("debian") => Self::Debian,
                    Some("fedora") => Self::Fedora,
                    Some("arch") => Self::Arch,
                    Some("centos") => Self::Centos,
                    Some("rhel") => Self::Redhat,
                    Some("linuxmint") => Self::Mint,
                    _ => Self::Tux,
                }
            }
        }
    }
}

/// A paired device's mark: its logo, and its number among the user's
/// devices of that logo, none before its runner first connected. The page
/// shows the number only while another device has the same logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceMark {
    pub logo: DeviceLogo,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    pub number: Option<u32>,
}

/// Whether a device's runner serves it: `online` while its runner is
/// connected, `updating` while it replaces itself with the backend's runner
/// release (`crates/runner/src/update.rs`), `offline` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Online,
    Updating,
    Offline,
}

/// A device as the web app sees it. `platform` is the one its runner
/// reported; `state` says whether its runner serves it now;
/// `download_percent` is how much of a runner executable this backend has
/// served it, while it serves one, as a device downloads its next runner
/// ahead or updates (`crates/runner/src/update.rs`), null otherwise; `home` is
/// the home directory it reported when it last connected, null until then
/// (the backend keeps it in memory only); `installed` is what its runner
/// last reported its artifact cache holds, kept while it is offline
/// (`crates/runner-command-packages/src/cache.rs`); `downloading` is what
/// it downloads into that cache now and how far each came, while its runner
/// is connected (the backend keeps it in memory only); `os` and `runner_version`
/// are the operating system and the runner release its runner last
/// reported, null before its runner first connected; `mark` is its mark,
/// null for the Cloud; `direct` is whether
/// `route` is how pages reach it (`packages/web/src/direct/index.ts`); `start_command` is what a person types in a terminal on a
/// paired device to start its runner again, null for the Cloud;
/// `autostart_unavailable` is why the device will not start its runner by
/// itself after a restart, as its runner last reported it, null when it
/// does or before its runner first said (`crates/runner/src/autostart.rs`).
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
    #[schemars(with = "Nullable<u8>")]
    pub download_percent: Option<u8>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub home: Option<String>,
    pub installed: Vec<HostArtifact>,
    pub downloading: Vec<HostDownload>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<OperatingSystem>")]
    pub os: Option<OperatingSystem>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub runner_version: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<DeviceMark>")]
    pub mark: Option<DeviceMark>,
    pub route: DeviceRoute,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub start_command: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<AutostartUnavailable>")]
    pub autostart_unavailable: Option<AutostartUnavailable>,
}

/// The commands that install and start the runner on a computer of each
/// system, which the Add Device dialog and `demi host install-command` show
/// as they are. They hold no credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstallCommands {
    pub linux: String,
    pub macos: String,
    pub windows: String,
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

/// `PATCH /devices/:id`: a paired device's new name, its route, or both;
/// each applies when present.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeDevice {
    #[serde(default)]
    #[garde(inner(length(chars, min = 1)))]
    pub name: Option<Trimmed>,
    #[serde(default)]
    #[garde(skip)]
    pub route: Option<DeviceRoute>,
}

/// `{ device }`: the answer of a claim and of a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceAnswer {
    pub device: DeviceDto,
}

/// `{ removed }`: the answer of a revocation, the ids of the projects
/// that went with the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RevokedDevice {
    pub removed: Vec<ProjectId>,
}

/// Which of the page's two peers with a device a signaling message is about
/// (`crates/runner-direct/src/lib.rs`): the one for files and calls,
/// or the one for live views, which the runner makes without SCTP's
/// congestion window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirectPeer {
    Files,
    Live,
}

/// A message the page sends on a device's direct channel signaling lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectRequest {
    /// The page's offer for a new peer, which replaces that peer.
    Offer {
        #[garde(skip)]
        peer: DirectPeer,
        #[garde(length(min = 1))]
        sdp: String,
    },
    /// A candidate the browser found after the peer's offer, the
    /// `candidate:` line as it gives it; the runner checks it too.
    Candidate {
        #[garde(skip)]
        peer: DirectPeer,
        #[garde(length(min = 1, max = CANDIDATE_CHARS))]
        candidate: String,
    },
    /// The page closed the peer: the runner closes its side.
    Close {
        #[garde(skip)]
        peer: DirectPeer,
    },
    /// A probe of the relay path, which measures it beside the `files`
    /// peer (`packages/web/src/direct/measure.ts`).
    Ping {
        #[garde(skip)]
        id: u32,
    },
}


/// A message the backend sends on a direct channel signaling lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DirectMessage {
    /// The runner's answer to the page's last offer for `peer`.
    Answer { peer: DirectPeer, sdp: String },
    /// A candidate the runner found after its answer, such as the address a
    /// STUN server saw it at; it may come before the answer.
    Candidate { peer: DirectPeer, candidate: String },
    /// The runner's answer to the page's probe `id`.
    Pong { id: u32 },
    /// The runner did not answer the page's last offer for `peer`.
    Unanswered { peer: DirectPeer, code: Unanswered },
    /// The runner closed `peer`, since the user turned a plugin on or off:
    /// the page makes a new offer for it at once, whose introduction carries
    /// the user's streams as they are now.
    Closed { peer: DirectPeer },
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
