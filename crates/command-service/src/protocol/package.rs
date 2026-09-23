//! Native command packages: the descriptor that identifies a release, the
//! artifact of each target, and where a runner fetches an artifact.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;
use sha2::{Digest, Sha256};

use super::ProtocolError;

/// The native command protocol version a descriptor declares.
pub const VERSION: u64 = 1;

/// The targets a published release carries.
pub const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];

/// The largest integer a JavaScript peer holds exactly.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// One target's executable: its SHA-256 and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PackageArtifact {
    #[garde(custom(digest))]
    pub sha256: String,
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub size: u64,
}

/// A native command package release: its identity, the operations it serves
/// and the artifact of each target it carries. Publication requires every
/// target; a development release may carry fewer.
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

    /// Whether a service's catalog is the one this descriptor declares: the
    /// same protocol and the same operations, in any order.
    pub fn serves(&self, info: &ServiceInfo) -> bool {
        let declared: HashSet<&String> = self.operations.iter().collect();
        let served: HashSet<&String> = info.operations.iter().collect();
        info.protocol_version == self.protocol_version && declared == served
    }
}

/// A URL the runner downloads an artifact from, valid until `expires_at`
/// (milliseconds since the Unix epoch) when set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactUrl {
    #[garde(url)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
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

/// Whether `value` is a SHA-256 digest in lowercase hexadecimal, the form of
/// every artifact and descriptor identity.
pub fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
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

/// Each key is a known target and each artifact is valid.
fn target_artifacts(targets: &BTreeMap<String, PackageArtifact>, _: &()) -> garde::Result {
    for (name, artifact) in targets {
        target(name, &())?;
        garde::Validate::validate(artifact)
            .map_err(|report| garde::Error::new(format!("{name}: {report}")))?;
    }
    Ok(())
}
