//! The runner release record (`builds-and-releases.md` § Packaging): one
//! release of `demi-runner`, the wire and command protocol versions it
//! speaks, and the executable of each target it carries. A runner release
//! directory's `manifest.json` holds it, and a Cloud image manifest embeds it
//! to name its runner. Beside it, a server release's own record,
//! `release.json`, which says where the release's files are, and the name
//! of each file (`builds-and-releases.md` § Server release). And the
//! release check before a runner's socket opens (`runner.md` § Runner
//! updates), the one part of the connection every release keeps.

use std::collections::BTreeMap;
use std::path::PathBuf;

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

/// A server release's record, the root's `release.json`: where the
/// release's files are, an HTTPS URL or an absolute directory on the
/// backend's machine (`native-runtime.md` § Backend deployment
/// configuration).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ServerRelease {
    #[garde(custom(files_location))]
    pub files: String,
}

/// The name of a server release's record in its root.
pub const SERVER_RELEASE: &str = "release.json";

/// The runner's executable, as a release's files name it.
pub const RUNNER: &str = "demi-runner";

/// Where a server release's files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesLocation {
    /// A URL that each file's name is appended to.
    Url(url::Url),
    Directory(PathBuf),
}

impl ServerRelease {
    /// Decodes a record's JSON and checks its values.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let release: Self = serde_json::from_slice(bytes)?;
        garde::Validate::validate(&release)
            .map_err(|report| ReleaseError::Invalid(report.to_string().trim_end().to_owned()))?;
        Ok(release)
    }

    /// Where the files are. A URL's path ends with `/`, so a file's name
    /// joins it as the last segment.
    pub fn location(&self) -> FilesLocation {
        match url::Url::parse(&self.files) {
            Ok(mut url) => {
                if !url.path().ends_with('/') {
                    let path = format!("{}/", url.path());
                    url.set_path(&path);
                }
                FilesLocation::Url(url)
            }
            Err(_) => FilesLocation::Directory(PathBuf::from(&self.files)),
        }
    }
}

/// A garde rule: an HTTPS URL without credentials, a query or a fragment,
/// or an absolute path.
fn files_location(value: &str, _: &()) -> garde::Result {
    match url::Url::parse(value) {
        Ok(url) => {
            let plain = url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none();
            if !plain {
                return Err(garde::Error::new(
                    "the release's files are at an HTTPS URL without credentials, a query or a fragment",
                ));
            }
            Ok(())
        }
        Err(_) if std::path::Path::new(value).is_absolute() => Ok(()),
        Err(_) => Err(garde::Error::new(
            "the release's files are at an HTTPS URL or an absolute directory",
        )),
    }
}

/// The name of `executable`'s file for `target` among a release's files,
/// such as `demi-runner-aarch64-apple-darwin` or
/// `demi-file-x86_64-pc-windows-msvc.exe`.
pub fn release_file(executable: &str, target: &str) -> String {
    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("{executable}-{target}{suffix}")
}

/// What a compressed copy's name adds to its file's.
pub const COMPRESSED_SUFFIX: &str = ".zst";

/// The name of the compressed copy of `executable`'s file for `target`,
/// which is how a release's files hold a command program and a runner.
pub fn compressed_file(executable: &str, target: &str) -> String {
    format!("{}{COMPRESSED_SUFFIX}", release_file(executable, target))
}

/// The variable that names the release a runner was installed from: an
/// installer's launcher sets it, and so does the machine manager for a
/// Cloud's runner.
pub const RELEASE_ENV: &str = "DEMI_RELEASE_ID";

/// The request header that names the release a runner was installed from.
pub const RELEASE_HEADER: &str = "demi-runner-release";

/// The request header that names a runner's target.
pub const TARGET_HEADER: &str = "demi-runner-target";

/// The request header that carries a paired runner's device token, by which
/// the backend knows which device a 409 sends to an update.
pub const TOKEN_HEADER: &str = "demi-runner-token";

/// The backend's 409 answer to a runner of another release than its own:
/// the backend's runner release and, when it carries one for the runner's
/// target, that executable. Every release reads this body, so fields may be
/// added to it but never changed or removed, and a reader ignores the
/// fields it does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
pub struct RunnerUpdate {
    #[garde(custom(digest))]
    pub release: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub executable: Option<UpdateExecutable>,
}

/// The executable a runner updates to: its SHA-256 and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
pub struct UpdateExecutable {
    #[garde(custom(digest))]
    pub sha256: String,
    #[garde(range(min = 1))]
    pub size: u64,
}

impl RunnerUpdate {
    /// Decodes a 409 answer's body and checks its values.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let update: Self = serde_json::from_slice(bytes)?;
        garde::Validate::validate(&update)
            .map_err(|report| ReleaseError::Invalid(report.to_string().trim_end().to_owned()))?;
        Ok(update)
    }
}
