//! The runner release record (`crates/xtask/src/native/package.rs`): one
//! release of `demi-runner`, the wire and command protocol versions it speaks,
//! and the executable of each target it carries.
//!
//! The record appears in these places:
//!
//! - A runner release directory's `manifest.json` holds it.
//! - A Cloud image manifest embeds it to name its runner.
//! - Beside it, a server release's `files/` holds each target's executable
//!   under the name the record gives it (`crates/xtask/src/server_release.rs`).
//! - The release check before a runner's socket opens, and the backend's
//!   management socket, which an upgrade's next release asks
//!   (`crates/server/src/main.rs`), are the one part of the connection every
//!   release keeps.
//!
//! # The release check
//!
//! A runner always runs the runner release of the backend it serves. Before its
//! socket opens, the backend compares the release in `RELEASE_HEADER` with its
//! current runner release (the one `runners/` names). It answers any other with
//! 409 and a [`RunnerUpdate`]: its release and, when it has one for the
//! runner's target (`TARGET_HEADER`), the executable's size and SHA-256. A
//! backend without runner releases, as in development, checks none.
//!
//! The check comes before the socket opens because the runner wire changes from
//! release to release: a runner of an earlier release could not read a refusal
//! written in a later one. The headers (`TOKEN_HEADER` names the device a 409
//! sends to an update) and the 409 body are therefore the one part of the
//! connection every release keeps: fields may be added to them, never changed
//! or removed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use demi_command_protocol::{PackageArtifact, digest, target_artifacts};
use serde::{Deserialize, Serialize};

use crate::wire;

/// A runner release. Publication requires every target; a development
/// release may carry fewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerRelease {
    /// The release's identity: the SHA-256 of its versions and targets.
    #[garde(custom(digest))]
    pub release: String,
    /// The wire and command protocol versions it speaks, which only a
    /// runtime that runs it must speak too ([`RunnerRelease::decode`]).
    #[garde(skip)]
    pub wire: u32,
    #[garde(skip)]
    pub command_protocol: u64,
    #[garde(custom(target_artifacts))]
    pub targets: BTreeMap<String, PackageArtifact>,
}

/// A record that is not a valid runner release.
#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error(transparent)]
    Shape(#[from] serde_json::Error),
    #[error("invalid runner release: {0}")]
    Invalid(String),
    #[error(
        "the runner release speaks wire {wire} and command protocol {command_protocol}, this build wire {} and command protocol {}",
        wire::VERSION,
        demi_command_protocol::VERSION
    )]
    Incompatible { wire: u32, command_protocol: u64 },
}

impl RunnerRelease {
    /// Decodes a release record's JSON for this build to run: intact, and
    /// speaking this build's wire and command protocol versions.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let release = Self::decode_intact(bytes)?;
        release.compatible()?;
        Ok(release)
    }

    /// Decodes a release record's JSON and checks its integrity alone, its
    /// digests and targets, whatever versions it speaks: the record of a
    /// release this build serves but does not run, such as the next one an
    /// upgrade names (`crates/server/src/main.rs`).
    pub fn decode_intact(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let release: Self = serde_json::from_slice(bytes)?;
        release.validate()?;
        Ok(release)
    }

    /// Checks the record's integrity: its digests and targets.
    pub fn validate(&self) -> Result<(), ReleaseError> {
        garde::Validate::validate(self)
            .map_err(|report| ReleaseError::Invalid(report.to_string().trim_end().to_owned()))
    }

    /// Checks that the release speaks this build's wire and command
    /// protocol versions.
    pub fn compatible(&self) -> Result<(), ReleaseError> {
        if self.wire == wire::VERSION && self.command_protocol == demi_command_protocol::VERSION {
            return Ok(());
        }
        Err(ReleaseError::Incompatible {
            wire: self.wire,
            command_protocol: self.command_protocol,
        })
    }
}

/// The directory of a server release's root that holds the release's files:
/// the compressed copy of each runner and command program for each target,
/// which the backend gives the Hosts (`crates/xtask/src/server_release.rs`).
pub const FILES: &str = "files";

/// The runner's executable, as a release's files name it.
pub const RUNNER: &str = "demi-runner";

/// The name of `executable`'s file for `target` among a release's files,
/// such as `demi-runner-aarch64-apple-darwin` or
/// `demi-file-x86_64-pc-windows-msvc.exe`.
pub fn release_file(executable: &str, target: &str) -> String {
    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("{executable}-{target}{suffix}")
}

/// What a compressed copy's name adds to its file's.
pub const COMPRESSED_SUFFIX: &str = ".zst";

/// The name of the compressed copy of `executable`'s file for `target`,
/// which is how a release's files hold a command program and a runner.
pub fn compressed_file(executable: &str, target: &str) -> String {
    format!("{}{COMPRESSED_SUFFIX}", release_file(executable, target))
}

/// The variable that names the release a runner was installed from: an
/// installer's launcher sets it, and so does the machine manager for a
/// Cloud's runner.
pub const RELEASE_ENV: &str = "DEMI_RELEASE_ID";

/// The request header that names the release a runner was installed from.
pub const RELEASE_HEADER: &str = "demi-runner-release";

/// The request header that names a runner's target.
pub const TARGET_HEADER: &str = "demi-runner-target";

/// The request header that carries a paired runner's device token, by which
/// the backend knows which device a 409 sends to an update.
pub const TOKEN_HEADER: &str = "demi-runner-token";

/// The backend's 409 answer to a runner of another release than its own:
/// the backend's runner release and, when it carries one for the runner's
/// target, that executable. Every release reads this body, so fields may be
/// added to it but never changed or removed, and a reader ignores the
/// fields it does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
pub struct RunnerUpdate {
    #[garde(custom(digest))]
    pub release: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub executable: Option<UpdateExecutable>,
}

/// The executable a runner updates to: its SHA-256 and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
pub struct UpdateExecutable {
    #[garde(custom(digest))]
    pub sha256: String,
    #[garde(range(min = 1))]
    pub size: u64,
}

impl RunnerUpdate {
    /// Decodes a 409 answer's body and checks its values.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let update: Self = serde_json::from_slice(bytes)?;
        garde::Validate::validate(&update)
            .map_err(|report| ReleaseError::Invalid(report.to_string().trim_end().to_owned()))?;
        Ok(update)
    }
}

/// The variable that names the backend's management socket, and its
/// default (`crates/server/src/main.rs`): only root may open it.
pub const MANAGE_SOCKET_ENV: &str = "DEMI_BACKEND_MANAGE_SOCKET";
pub const MANAGE_SOCKET: &str = "/run/demi-backend-manage/manage.sock";

/// The variable that names the file an administrator's Update Now writes
/// the version to, and that file on a server: the backend's unit names it,
/// a path unit starts `demi-server upgrade --requested` when it appears, and
/// a backend without it offers no update.
pub const UPGRADE_REQUEST_ENV: &str = "DEMI_UPGRADE_REQUEST";
pub const UPGRADE_REQUEST: &str = "/run/demi-upgrade/version";

/// A line `demi-server` writes on the management socket while it upgrades
/// the server, its own connection each: how far the upgrade is. The
/// backend shows it to the administrators, and answers a line of its own,
/// any line, once it has acted on it: for `restarting`, once every page has
/// heard that the backend stops.
///
/// A line is written by the running release's `demi-server` while it
/// downloads, and by the next release's from its preparation on, to a
/// backend of the running release: steps and fields may be added but never
/// changed or removed, and a backend answers a line it cannot read as it
/// answers any, which the writer ignores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "upgrade", rename_all = "snake_case")]
pub enum UpgradeNotice {
    /// The release downloads: `bytes_done` of `bytes`, at `bytes_per_second`
    /// of late, 0 before it is measured.
    Downloading {
        to: String,
        bytes_done: u64,
        bytes: u64,
        #[serde(default)]
        bytes_per_second: u64,
    },
    /// The release checks itself and its devices fetch its runner, while
    /// the backend still serves.
    Preparing { to: String },
    /// The backend stops next.
    Restarting { to: String },
    /// The upgrade did not happen, and the server runs the release it ran.
    Failed { to: String, reason: String },
}

impl UpgradeNotice {
    /// Decodes a management line that is a notice; none for another line,
    /// as a prefetch request is.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        value.get("upgrade")?;
        serde_json::from_value(value).ok()
    }
}

/// The management socket's one request, a line of JSON: the runner release the
/// next server release installs, and that release's root, whose `runners/`
/// holds its record and whose `files/` its executables.
///
/// The backend has its connected paired devices' runners download the release
/// ahead and answers a [`PrefetchProgress`] line each time one of them moves
/// on, until none is downloading.
///
/// The request is written by the next release's `demi-server` and read by the
/// running release's backend, so fields may be added but never changed or
/// removed, and a reader ignores the fields it does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
pub struct PrefetchRequest {
    #[garde(custom(digest))]
    pub release: String,
    #[garde(custom(absolute))]
    pub root: PathBuf,
}

impl PrefetchRequest {
    /// Decodes a request line and checks its values.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let request: Self = serde_json::from_slice(bytes)?;
        garde::Validate::validate(&request)
            .map_err(|report| ReleaseError::Invalid(report.to_string().trim_end().to_owned()))?;
        Ok(request)
    }
}

/// A garde rule: an absolute path.
fn absolute(value: &std::path::Path, _: &()) -> garde::Result {
    if value.is_absolute() {
        Ok(())
    } else {
        Err(garde::Error::new("the release's root is an absolute directory"))
    }
}

/// A line the management socket answers a [`PrefetchRequest`] with: how far
/// the devices are, or why the backend asks none of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrefetchAnswer {
    Progress(PrefetchProgress),
    Refused { error: String },
}

/// Where each device the backend asked to download ahead is, by the name
/// its user gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefetchProgress {
    pub devices: Vec<DevicePrefetch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePrefetch {
    pub name: String,
    #[serde(flatten)]
    pub state: PrefetchState,
}

/// A device's download ahead: running, its executable in place, or failed
/// for a reason, after which the device downloads after the switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PrefetchState {
    Downloading,
    Done,
    Failed { reason: String },
}

impl PrefetchProgress {
    /// Whether no device is downloading any more.
    pub fn finished(&self) -> bool {
        self.devices
            .iter()
            .all(|device| device.state != PrefetchState::Downloading)
    }
}
