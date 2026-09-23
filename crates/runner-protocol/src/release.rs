//! The runner release record (`builds-and-releases.md` § Packaging): one
//! release of `demi-runner`, the wire and command protocol versions it
//! speaks, and the executable of each target it carries. A runner release
//! directory's `manifest.json` holds it, and a Cloud image manifest embeds it
//! to name its runner.

use std::collections::BTreeMap;

use demi_command_service::protocol::{PackageArtifact, digest, target_artifacts};
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
    #[garde(range(equal = demi_command_service::protocol::VERSION))]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A release record as `scripts/native/release-runner.ts` writes it.
    const RECORD: &str = r#"{"release":"317dd84e2ce0846a1bea4bc5959959c04af7ba8e4de32b3752fdd6b409f7b1a5","wire":18,"commandProtocol":1,"targets":{"aarch64-apple-darwin":{"sha256":"dbf18cb3af50a3348a834ea9cee7981f7354f84aa89764f4d68812c81ebe045b","size":38772096},"aarch64-unknown-linux-musl":{"sha256":"5d4219232ad6a95b2a0e72097011e91ae3773e8523e8032788904fa0e9164197","size":38710848}}}"#;

    #[test]
    fn a_release_record_decodes_and_encodes_as_the_typescript_writes_it() {
        let release = RunnerRelease::decode(RECORD.as_bytes()).expect("valid record");
        assert_eq!(release.targets["aarch64-unknown-linux-musl"].size, 38_710_848);
        assert_eq!(serde_json::to_string(&release).unwrap(), RECORD);
    }

    #[test]
    fn a_release_record_is_checked_in_every_field() {
        for (from, to) in [
            (r#""release":"317dd84e"#, r#""release":"317DD84E"#),
            (r#""wire":18"#, r#""wire":17"#),
            (r#""commandProtocol":1"#, r#""commandProtocol":2"#),
            ("aarch64-apple-darwin", "aarch64-apple-ios"),
            (r#""size":38772096"#, r#""size":0"#),
            (r#""wire":18"#, r#""wire":18,"channel":"beta""#),
        ] {
            let record = RECORD.replacen(from, to, 1);
            assert!(RunnerRelease::decode(record.as_bytes()).is_err(), "{record}");
        }
    }
}
