//! The pinned Chrome for Testing release (`browser.md` § Browser
//! distribution). Nothing resolves a moving channel: a release names each
//! platform's archive by URL, size and digest, which `demi browser install`
//! downloads from its official URL. Beside it, the record of what Chrome
//! needs on Linux that Demi does not install
//! (`builds-and-releases.md` § Chrome for Testing).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::DecodeError;

/// The artifact line the pinned release is installed as on a Host, which
/// its installed artifacts name (`native-runtime.md` § Installed
/// artifacts).
pub const ARTIFACT: &str = "Chrome for Testing";

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

    /// The release's title for the user, such as `Chrome for Testing
    /// 153.0.8010.36`.
    pub fn title(&self) -> String {
        format!("{ARTIFACT} {}", self.version)
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

/// What Chrome needs on Linux that Demi does not install: the shared
/// libraries it loads that a minimal Ubuntu lacks, and the fonts pages need
/// to show their text, each with the Ubuntu package that provides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LinuxRequirements {
    #[garde(dive)]
    pub libraries: Vec<LinuxLibrary>,
    #[garde(dive)]
    pub fonts: Vec<LinuxFont>,
}

impl LinuxRequirements {
    /// The record this build of Demi carries, `release/linux.json` beside
    /// this module.
    pub fn pinned() -> Result<Self, DecodeError> {
        demi_shared_types::decode_slice(include_bytes!("release/linux.json"))
    }
}

/// A shared library by its soname, such as `libnss3.so`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LinuxLibrary {
    #[garde(length(min = 1))]
    pub name: String,
    #[garde(length(min = 1))]
    pub package: String,
}

/// A font a page needs for `purpose`, such as `color emoji`, found by the
/// name of its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct LinuxFont {
    #[garde(length(min = 1))]
    pub purpose: String,
    #[garde(length(min = 1))]
    pub file: String,
    #[garde(length(min = 1))]
    pub package: String,
}
