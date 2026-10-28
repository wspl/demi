//! Settings › Updates: which release the server runs, the newest of
//! its channel, and an update a master or an administrator runs
//! (`crates/server/src/main.rs`). Only a server that `demi-server` installed
//! offers it.

use demi_shared_types::{MAX_SAFE_INTEGER, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use demi_runner_protocol::release::UpgradeStep;
use serde::{Deserialize, Serialize};

/// `GET /server-upgrade`, and what its changes answer; the state channel
/// pushes it to every master's and administrator's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerUpgrade {
    /// The release the server runs.
    pub running: String,
    pub channel: UpgradeChannel,
    pub check: UpgradeCheck,
    /// The newest release, which the server gets ready while it serves.
    pub preparation: UpgradePreparation,
    pub run: UpgradeRun,
}

/// `GET /server-upgrade/agents`: the agents at work on the server, every
/// account's, which an update interrupts until the server serves again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkingAgents {
    pub count: u32,
}

/// Which releases the server is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeChannel {
    /// The releases Demi publishes, which GitHub marks *latest* in turn.
    Beta,
    /// Every release, the pre-releases a deploy publishes included.
    Canary,
}

serde_plain::derive_display_from_serialize!(UpgradeChannel);
serde_plain::derive_fromstr_from_deserialize!(UpgradeChannel);

/// What the last look for a newer release found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UpgradeCheck {
    /// The backend has not looked since it started.
    Never,
    /// It looks now.
    Checking,
    Checked {
        at: Timestamp,
        /// The newest release of the channel when it is newer than the
        /// running one.
        newer: Option<String>,
        /// The page of `newer`'s release, with its notes.
        notes_url: Option<String>,
    },
    Failed { at: Timestamp, reason: String },
}

/// How far the server is in getting a newer release ready ahead of Update
/// Now: downloaded and unpacked, its gVisor and Cloud image installed, and
/// its runner on the devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UpgradePreparation {
    /// No newer release is being or has been got ready.
    None,
    /// Its archives download.
    Downloading {
        version: String,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        done: u64,
        #[garde(range(max = MAX_SAFE_INTEGER))]
        total: u64,
    },
    /// It unpacks, and its gVisor and Cloud image install.
    Installing { version: String },
    /// Update Now has only the checks and the restart left.
    Ready { version: String },
    /// The work failed; the next check tries again.
    Failed { version: String, reason: String },
}

/// An update that runs or did not happen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UpgradeRun {
    None,
    /// It runs: every step of it while the server serves, in order.
    Updating { to: String, steps: Vec<UpgradeStep> },
    /// The update did not happen; the server runs the release it ran. The
    /// steps say which one failed; none when it failed before it knew what
    /// to move to.
    Failed {
        to: String,
        reason: String,
        steps: Vec<UpgradeStep>,
    },
}

/// `PUT /server-upgrade/channel`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct SetUpgradeChannel {
    #[garde(skip)]
    pub channel: UpgradeChannel,
}

/// `POST /server-upgrade/run`: the release the page offered, which the
/// update moves to only while it is still the newest of the channel.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct RunUpgrade {
    #[garde(length(chars, min = 1, max = 64))]
    pub version: String,
}
