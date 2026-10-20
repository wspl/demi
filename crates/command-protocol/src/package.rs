//! Command packages: the descriptor that identifies a release, the
//! artifact of each target, and where a runner fetches an artifact.
//!
//! A command declaration names a package and an operation; the backend's
//! startup catalog maps each package to one release descriptor; a job's
//! manifest pins that descriptor's hash and the operation. The pin is what
//! keeps a later release or download URL from changing what an existing job
//! executes. The catalog is fixed for the backend's lifetime (a new release
//! takes effect when it restarts) and refuses duplicate ids, incomplete
//! packages and unknown operations when it is built; declarations never
//! import a compiled-in descriptor.
//!
//! A descriptor's identity is the hash of its canonical JSON (RFC 8785).
//! Where its executables are stored, and the object keys and URLs that
//! locate them, take no part in it. A package declares its operations once,
//! as the operation list its contract crate exports, so the descriptor, the
//! service's own `GET /v1/info` and the handler's routing cannot list
//! different sets. The runner validates descriptors and references itself
//! and never repairs arguments or falls back to another version.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;
use sha2::{Digest, Sha256};

use super::ProtocolError;

/// The native command protocol version a descriptor declares.
pub const VERSION: u64 = 1;

/// The targets a published release carries. A published version supplies the
/// same operations on all of them, so a job can pick its Host without meeting
/// a platform-specific gap. Completeness is a rule of the published release,
/// not of a descriptor or the object store: packaging writes all six unless
/// told otherwise and the release workflow publishes only that, while a
/// developer's release carries just the targets of the Hosts in use, and the
/// backend serves any nonempty set the same way. A Host whose target a
/// release lacks fails the command with a catalog mismatch and cannot install
/// its runner from that release; nothing falls back to another target.
pub const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];

/// The largest integer a JavaScript peer holds exactly, as core's
/// `MAX_SAFE_INTEGER`. This crate keeps its own: every command program links
/// it, and it depends on no other crate of the workspace, core included.
pub(crate) const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// One target's executable: its SHA-256 and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PackageArtifact {
    #[garde(custom(digest))]
    pub sha256: String,
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub size: u64,
}

/// A command package release: its identity, the operations it serves and
/// the executable of each target it carries. Software the program installs
/// itself, such as Chrome for Testing, is the program's own and not the
/// release's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDescriptor {
    #[garde(pattern(r"^[a-z0-9]+(?:[.-][a-z0-9]+)+$"))]
    pub id: String,
    #[garde(length(min = 1))]
    pub version: String,
    #[garde(range(equal = VERSION))]
    pub protocol_version: u64,
    #[garde(length(min = 1), inner(length(min = 1)), custom(unique))]
    pub operations: Vec<String>,
    #[garde(custom(target_artifacts))]
    pub targets: BTreeMap<String, PackageArtifact>,
}

impl PackageDescriptor {
    pub fn parse(value: serde_json::Value) -> Result<Self, ProtocolError> {
        let descriptor: Self = serde_json::from_value(value)?;
        garde::Validate::validate(&descriptor)?;
        Ok(descriptor)
    }

    /// The descriptor's identity: the SHA-256 of its canonical JSON.
    pub fn digest(&self) -> Result<String, ProtocolError> {
        garde::Validate::validate(self)?;
        canonical_digest(self)
    }

    /// The executable of `target`, when its SHA-256 is `sha256`.
    pub fn carries(&self, target: &str, sha256: &str) -> Option<PackageArtifact> {
        self.targets
            .get(target)
            .filter(|artifact| artifact.sha256 == sha256)
            .cloned()
    }

    /// Whether a service's catalog is the one this descriptor declares: the
    /// same protocol and the same operations, in any order.
    pub fn serves(&self, info: &ServiceInfo) -> bool {
        let declared: HashSet<&String> = self.operations.iter().collect();
        let served: HashSet<&String> = info.operations.iter().collect();
        info.protocol_version == self.protocol_version && declared == served
    }
}

/// A URL the runner downloads an artifact from, valid until `expires_at`
/// (milliseconds since the Unix epoch) when set. It is an HTTP or HTTPS URL
/// without credentials: object storage answers with a signed HTTPS URL, and a
/// local store with a URL on the backend itself. The scheme cannot
/// change what runs, because the runner checks the download against the size
/// and SHA-256 its pinned descriptor declares (`crates/runner-command-packages/src/cache.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactUrl {
    #[garde(custom(download_url))]
    pub url: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub expires_at: Option<i64>,
}

/// A path on the runner's machine that holds an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPath {
    #[garde(length(min = 1))]
    pub path: String,
}

/// Where a runner fetches an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(untagged)]
pub enum ArtifactLocation {
    Url(#[garde(dive)] ArtifactUrl),
    Path(#[garde(dive)] ArtifactPath),
}

/// What a resident service answers on its info path: the protocol it speaks
/// and the operations it serves, each once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceInfo {
    #[garde(range(equal = VERSION))]
    pub protocol_version: u64,
    #[garde(length(min = 1), inner(length(min = 1)), custom(unique))]
    pub operations: Vec<String>,
}

impl ServiceInfo {
    /// Checks a catalog a service offers or a runner received.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        garde::Validate::validate(self)?;
        Ok(())
    }
}

pub(crate) fn download_url(value: &str, _: &()) -> garde::Result {
    let url = url::Url::parse(value).map_err(|error| garde::Error::new(error.to_string()))?;
    let web = matches!(url.scheme(), "http" | "https");
    if !web || !url.username().is_empty() || url.password().is_some() {
        return Err(garde::Error::new(
            "artifact downloads require an HTTP or HTTPS URL without credentials",
        ));
    }
    Ok(())
}

/// Whether `value` is a SHA-256 digest in lowercase hexadecimal, the form of
/// every artifact and descriptor identity.
pub fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `target` is one of the [`TARGETS`].
pub fn is_target(target: &str) -> bool {
    TARGETS.contains(&target)
}

/// A garde rule: the value is a SHA-256 digest ([`is_digest`]).
pub fn digest(value: &str, _: &()) -> garde::Result {
    if !is_digest(value) {
        return Err(garde::Error::new("is not a SHA-256 digest"));
    }
    Ok(())
}

/// A garde rule: the value is one of the [`TARGETS`].
pub fn target(value: &str, _: &()) -> garde::Result {
    if !is_target(value) {
        return Err(garde::Error::new(format!("unknown target {value}")));
    }
    Ok(())
}

pub fn canonical_digest<T: serde::Serialize>(value: &T) -> Result<String, ProtocolError> {
    let bytes = serde_json_canonicalizer::to_vec(value)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn target_artifact<'a>(
    package: &'a PackageDescriptor,
    target: &str,
) -> Result<&'a PackageArtifact, ProtocolError> {
    package
        .targets
        .get(target)
        .ok_or_else(|| ProtocolError::MissingTarget(target.to_owned()))
}

/// The build target used by native artifacts and their runtime dependencies.
pub fn host_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-musl";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-musl";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return "aarch64-pc-windows-msvc";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "x86_64-pc-windows-msvc";
    #[cfg(not(any(
        all(
            target_os = "macos",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(
            target_os = "linux",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(
            target_os = "windows",
            any(target_arch = "aarch64", target_arch = "x86_64")
        )
    )))]
    compile_error!("unsupported runner target");
}

fn unique(operations: &[String], _: &()) -> garde::Result {
    let mut seen = HashSet::new();
    match operations.iter().find(|operation| !seen.insert(*operation)) {
        Some(operation) => Err(garde::Error::new(format!(
            "operation {operation} appears twice"
        ))),
        None => Ok(()),
    }
}

/// A garde rule: an entry is a relative path of normal components.
pub(crate) fn archive_entry(value: &str, _: &()) -> garde::Result {
    let normal = !value.is_empty()
        && value
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..");
    if !normal || value.contains('\\') {
        return Err(garde::Error::new(format!(
            "{value} is not a relative path inside the archive"
        )));
    }
    Ok(())
}

/// A garde rule: each key is a known target and each artifact is valid, as
/// in a release's map of target artifacts.
pub fn target_artifacts(targets: &BTreeMap<String, PackageArtifact>, _: &()) -> garde::Result {
    for (name, artifact) in targets {
        target(name, &())?;
        garde::Validate::validate(artifact)
            .map_err(|report| garde::Error::new(format!("{name}: {report}")))?;
    }
    Ok(())
}
