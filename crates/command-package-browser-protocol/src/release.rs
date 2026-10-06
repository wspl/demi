//! The pinned Chrome for Testing release, and the Chrome runtime release
//! Linux Hosts start it with (`browser.md` § Browser distribution). Nothing
//! resolves a moving channel: a release names each platform's archive, and
//! the runtime's archives, by URL, size and digest, which `demi browser
//! install` downloads (`builds-and-releases.md` § Chrome for Testing,
//! § Chrome runtime).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use demi_shared_types::DecodeError;

/// The artifact line the pinned release is installed as on a Host, which
/// its installed artifacts name (`native-runtime.md` § Installed
/// artifacts).
pub const ARTIFACT: &str = "Chrome for Testing";

/// The artifact lines the Chrome runtime's archives are installed as: its
/// libraries for the Host's architecture, and its fonts.
pub const RUNTIME_LIBRARIES: &str = "Chrome runtime libraries";
pub const RUNTIME_FONTS: &str = "Chrome runtime fonts";

/// The file of each runtime archive that names its installation: a library
/// in the directory of the libraries, which holds `gio/modules` too, and
/// the fontconfig file, which finds the fonts relative to itself
/// (`builds-and-releases.md` § Chrome runtime).
pub const RUNTIME_LIBRARIES_ENTRY: &str = "lib/libnss3.so";
pub const RUNTIME_FONTS_ENTRY: &str = "fontconfig/fonts.conf";

/// One Chrome version and its archive for each platform, with the Chrome
/// runtime release that runs it on Linux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct BrowserRelease {
    /// Chrome's four-part version, such as `153.0.8010.36`.
    #[garde(pattern(r"^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$"))]
    pub version: String,
    #[garde(dive)]
    pub platforms: Vec<ReleasePlatform>,
    #[garde(dive)]
    pub runtime: ChromeRuntime,
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

    /// The runtime's archives Chrome starts with on `target`: on a Linux
    /// target, its libraries and its fonts; none elsewhere, where Chrome
    /// brings what it needs.
    pub fn runtime_archives(&self, target: &str) -> Option<RuntimeArchives<'_>> {
        let libraries = self.runtime.libraries.get(target)?;
        Some(RuntimeArchives {
            libraries,
            fonts: &self.runtime.fonts,
        })
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

/// The Chrome runtime release a Chrome release runs with on Linux: the
/// libraries' archive of each Linux target, and the fonts' archive they all
/// share (`builds-and-releases.md` § Chrome runtime).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ChromeRuntime {
    /// The runtime's release number, such as `1`.
    #[garde(range(min = 1))]
    pub release: u32,
    /// By the Rust target triple each runs on.
    #[garde(dive)]
    pub libraries: BTreeMap<String, RuntimeArchive>,
    #[garde(dive)]
    pub fonts: RuntimeArchive,
}

impl ChromeRuntime {
    /// The runtime's title for the user, such as `Chrome runtime 1`, which
    /// a sentence names as `the Chrome runtime 1`.
    pub fn title(&self) -> String {
        format!("Chrome runtime {}", self.release)
    }
}

/// One archive of the Chrome runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct RuntimeArchive {
    #[garde(url)]
    pub url: String,
    #[garde(range(min = 1))]
    pub size: u64,
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub sha256: String,
}

/// The runtime's two archives for one target.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeArchives<'a> {
    pub libraries: &'a RuntimeArchive,
    pub fonts: &'a RuntimeArchive,
}
