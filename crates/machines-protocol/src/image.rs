//! The Cloud image manifest (`images.md` § Release artifacts): what a base
//! archive holds and the build inputs it was made from. The packaging
//! command validates a manifest with it before publishing, and the manager
//! validates a release with it before importing. The SHA-256 of the exact
//! manifest bytes is the base version.

use std::collections::BTreeMap;

use demi_command_service::protocol::{PackageArtifact, PackageDescriptor, digest};
use demi_runner_protocol::release::RunnerRelease;
use serde::{Deserialize, Serialize};

/// The manifest's schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct FormatVersion;

impl TryFrom<u32> for FormatVersion {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, String> {
        if value != 1 {
            return Err(format!("unknown image format version {value}"));
        }
        Ok(Self)
    }
}

impl From<FormatVersion> for u32 {
    fn from(_: FormatVersion) -> Self {
        1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
}

/// A Cloud image's CPU architecture. Persisted state never moves between
/// architectures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    Amd64,
    Arm64,
}

impl Architecture {
    /// The architecture this program was built for, if Cloud images exist
    /// for it.
    pub fn host() -> Option<Self> {
        if cfg!(target_arch = "aarch64") {
            Some(Self::Arm64)
        } else if cfg!(target_arch = "x86_64") {
            Some(Self::Amd64)
        } else {
            None
        }
    }

    /// The native target of the image's executables.
    pub fn target(self) -> &'static str {
        match self {
            Self::Amd64 => "x86_64-unknown-linux-musl",
            Self::Arm64 => "aarch64-unknown-linux-musl",
        }
    }
}

/// The name of the base archive in the release directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootfsFile {
    #[serde(rename = "rootfs.tar.zst")]
    TarZst,
}

impl RootfsFile {
    pub fn name(self) -> &'static str {
        match self {
            Self::TarZst => "rootfs.tar.zst",
        }
    }
}

/// The base archive: its SHA-256, its byte size and its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct RootfsArchive {
    #[garde(custom(digest))]
    pub sha256: String,
    #[garde(range(min = 1))]
    pub size: u64,
    #[garde(skip)]
    pub file: RootfsFile,
}

/// A system package the image's dpkg database lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct InstalledPackage {
    #[garde(length(min = 1))]
    pub name: String,
    #[garde(length(min = 1))]
    pub version: String,
}

/// A standalone tool the build installed, such as uv or Chrome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct StandaloneTool {
    #[garde(length(min = 1))]
    pub name: String,
    #[garde(length(min = 1))]
    pub version: String,
    #[garde(custom(digest))]
    pub sha256: String,
}

/// A Cloud image release's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudImageManifest {
    #[garde(skip)]
    pub format_version: FormatVersion,
    #[garde(skip)]
    pub os: Os,
    #[garde(skip)]
    pub architecture: Architecture,
    #[garde(dive)]
    pub rootfs: RootfsArchive,
    /// The Ubuntu release of the base.
    #[garde(length(min = 1))]
    pub ubuntu: String,
    #[garde(dive)]
    pub packages: Vec<InstalledPackage>,
    /// The executables the image embeds, by absolute path under `/usr` or
    /// `/opt`, with their size and SHA-256.
    #[garde(custom(image_paths), dive)]
    pub executables: BTreeMap<String, PackageArtifact>,
    /// The command package releases whose artifacts the image embeds.
    #[garde(dive)]
    pub releases: Vec<PackageDescriptor>,
    /// The runner release of `/usr/bin/demi-runner`.
    #[garde(dive)]
    pub runner: RunnerRelease,
    #[garde(dive)]
    pub tools: Vec<StandaloneTool>,
}

/// A manifest the manager or the packaging command refuses.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error(transparent)]
    Shape(#[from] serde_json::Error),
    #[error("invalid Cloud image manifest: {0}")]
    Invalid(String),
    #[error("Runner release must identify the embedded executable")]
    Runner,
    #[error("Missing embedded artifact for {0}")]
    Release(String),
}

/// Where the runner executable lives in every image.
pub const RUNNER_PATH: &str = "/usr/bin/demi-runner";

impl CloudImageManifest {
    /// Decodes a manifest's bytes and checks it.
    pub fn decode(bytes: &[u8]) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Checks the manifest's values, then that its runner release names the
    /// embedded runner and that each command package's artifact for the
    /// image's target is embedded under its content-addressed path.
    pub fn validate(&self) -> Result<(), ManifestError> {
        garde::Validate::validate(self)
            .map_err(|report| ManifestError::Invalid(report.to_string().trim_end().to_owned()))?;
        let target = self.architecture.target();
        let runner = self.runner.targets.get(target);
        let embedded = self.executables.get(RUNNER_PATH);
        if runner.is_none() || runner != embedded {
            return Err(ManifestError::Runner);
        }
        for release in &self.releases {
            let Some(artifact) = release.targets.get(target) else {
                return Err(ManifestError::Release(release.id.clone()));
            };
            let prefix = format!("/opt/demi/artifacts/{}/", artifact.sha256);
            let present = self
                .executables
                .iter()
                .any(|(path, entry)| path.starts_with(&prefix) && entry == artifact);
            if !present {
                return Err(ManifestError::Release(release.id.clone()));
            }
        }
        Ok(())
    }
}

/// A garde rule: each executable is an absolute path under `/usr` or
/// `/opt`, on one line, with no `..` segment.
fn image_paths(executables: &BTreeMap<String, PackageArtifact>, _: &()) -> garde::Result {
    for path in executables.keys() {
        let under = path.starts_with("/usr/") || path.starts_with("/opt/");
        let named = path.len() > "/usr/".len() && !path.contains(['\r', '\n']);
        if !under || !named || path.split('/').any(|segment| segment == "..") {
            return Err(garde::Error::new(format!("invalid image executable path {path:?}")));
        }
    }
    Ok(())
}
