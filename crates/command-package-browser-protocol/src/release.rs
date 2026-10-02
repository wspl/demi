//! The pinned Chrome for Testing release (`browser.md` § Browser
//! distribution). Nothing resolves a moving channel: a release names each
//! platform's archive by URL, size and digest, and packaging makes it the
//! `chrome` resource of `demi.browser`'s releases.

use serde::{Deserialize, Serialize};

use demi_shared_types::DecodeError;

/// The resource of `demi.browser` that the pinned release is, whose entry
/// is Chrome's executable (`native-runtime.md` § Bind an exact package).
pub const RESOURCE: &str = "chrome";

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
        demi_shared_types::decode_slice(json.as_bytes())
    }

    /// The release this build of Demi pins, whose record is
    /// `release/chrome.json` beside this module.
    pub fn pinned() -> Result<Self, DecodeError> {
        Self::parse(include_str!("release/chrome.json"))
    }

    /// The resource's title for the user, such as `Chrome for Testing
    /// 153.0.8010.36`.
    pub fn title(&self) -> String {
        format!("Chrome for Testing {}", self.version)
    }

    /// The archive for `target`, when the release has one.
    pub fn platform(&self, target: &str) -> Option<&ReleasePlatform> {
        self.platforms
            .iter()
            .find(|platform| platform.target == target)
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
