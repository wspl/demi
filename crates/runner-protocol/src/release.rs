//! The runner release record (`builds-and-releases.md` § Packaging): one
//! release of `demi-runner`, the wire and command protocol versions it
//! speaks, and the executable of each target it carries. A runner release
//! directory's `manifest.json` holds it, and a Cloud image manifest embeds it
//! to name its runner.

use std::collections::BTreeMap;

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
    #[garde(range(equal = wire::VERSION))]
    pub wire: u32,
    #[garde(range(equal = demi_command_protocol::VERSION))]
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
}

impl RunnerRelease {
    /// Decodes a release record's JSON and checks its values.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let release: Self = serde_json::from_slice(bytes)?;
        release.validate()?;
        Ok(release)
    }

    /// Checks the record's values: its digests, versions and targets.
    pub fn validate(&self) -> Result<(), ReleaseError> {
        garde::Validate::validate(self)
            .map_err(|report| ReleaseError::Invalid(report.to_string().trim_end().to_owned()))
    }
}
