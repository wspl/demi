//! The pinned Chrome for Testing release and what an installation records
//! (`browser.md` § Browser distribution). Installers never resolve a moving
//! channel: a release names each platform's archive by URL, size and digest.

use serde::{Deserialize, Serialize};

use crate::DecodeError;

/// One Chrome version and its archive for each platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct BrowserRelease {
    /// Chrome's four-part version, such as `153.0.8010.36`.
    #[garde(pattern(r"^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$"))]
    pub version: String,
    #[garde(dive)]
    pub platforms: Vec<ReleasePlatform>,
}

impl BrowserRelease {
    /// Decodes a release record.
    pub fn parse(json: &str) -> Result<Self, DecodeError> {
        crate::decode_slice(json.as_bytes())
    }
}

/// One platform's archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ReleasePlatform {
    /// The Rust target triple the archive runs on.
    #[garde(length(min = 1))]
    pub target: String,
    #[garde(url)]
    pub url: String,
    #[garde(range(min = 1))]
    pub size: u64,
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub sha256: String,
    /// The executable's path inside the archive.
    #[garde(length(min = 1))]
    pub executable: String,
}

/// What an installation records beside its files: the SHA-256 of the archive
/// it came from and of its executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserInstallation {
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub archive_hash: String,
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub executable_hash: String,
}

impl BrowserInstallation {
    /// Decodes an installation's record.
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        crate::decode_slice(bytes)
    }
}
