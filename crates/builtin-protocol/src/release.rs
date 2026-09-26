//! The pinned Chrome for Testing release (`browser.md` § Browser
//! distribution). Installers never resolve a moving channel: a release names
//! each platform's archive by URL, size and digest, and `artifact`'s archive
//! installation installs it.

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

    /// The release this build of Demi pins, whose record is
    /// `release/chrome.json` beside this module.
    pub fn pinned() -> Result<Self, DecodeError> {
        Self::parse(include_str!("release/chrome.json"))
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
