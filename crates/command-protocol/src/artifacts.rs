//! The artifacts stream (`native-runtime.md` § The artifacts stream): a
//! program's requests that the runner install an artifact for one of its
//! invocations, or say which artifacts of a line it has, and their answers.

use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::ProtocolError;
use super::package::{MAX_SAFE_INTEGER, digest};

/// How an artifact is installed: one executable file, or a zip archive
/// unpacked, whose entry is the file its user starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactForm {
    File,
    Archive {
        #[garde(custom(super::package::archive_entry))]
        entry: String,
    },
}

/// An artifact to install for `invocation`: its line's name and its version
/// for the user, its bytes' SHA-256 and size, its form, and, for software the
/// program installs from its official source, the URL it downloads from
/// (`native-runtime.md` § The artifacts stream). Without a URL, the runner
/// asks the backend where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactInstall {
    #[garde(length(min = 1, max = 200))]
    pub invocation: String,
    #[garde(length(min = 1, max = 100))]
    pub name: String,
    #[garde(length(min = 1, max = 100))]
    pub version: String,
    #[garde(custom(digest))]
    pub sha256: String,
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub size: u64,
    #[garde(dive)]
    pub form: ArtifactForm,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(custom(crate::package::download_url)))]
    pub url: Option<String>,
}

/// Which artifacts of the line `name` the Host has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct ArtifactsInstalled {
    #[garde(length(min = 1, max = 100))]
    pub name: String,
}

/// One request a service writes as a standard output record: `id`, its own,
/// unique among its requests in flight, and either an install or a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRequest {
    #[garde(skip)]
    pub id: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(dive)]
    pub install: Option<ArtifactInstall>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(dive)]
    pub installed: Option<ArtifactsInstalled>,
}

/// What a request asks, once it carries exactly one of its kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactAsk {
    Install(ArtifactInstall),
    Installed(ArtifactsInstalled),
}

impl ArtifactRequest {
    pub fn new(id: u64, ask: ArtifactAsk) -> Self {
        let (install, installed) = match ask {
            ArtifactAsk::Install(install) => (Some(install), None),
            ArtifactAsk::Installed(installed) => (None, Some(installed)),
        };
        Self {
            id,
            install,
            installed,
        }
    }

    /// What the request asks; one that carries both kinds or neither is
    /// invalid.
    pub fn ask(&self) -> Result<ArtifactAsk, ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)?;
        match (&self.install, &self.installed) {
            (Some(install), None) => Ok(ArtifactAsk::Install(install.clone())),
            (None, Some(installed)) => Ok(ArtifactAsk::Installed(installed.clone())),
            _ => Err(ProtocolError::Invalid(
                "an artifact request carries either an install or a question".into(),
            )),
        }
    }
}

/// One artifact of a line the Host has: its version and SHA-256, and the
/// absolute path of the file or of the archive's entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledArtifact {
    #[garde(length(min = 1))]
    pub version: String,
    #[garde(custom(digest))]
    pub sha256: String,
    #[garde(length(min = 1))]
    pub path: String,
}

/// What a request was answered, once valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactReply {
    /// The installed artifact's path.
    Path(String),
    /// The line's artifacts, the newest install first.
    Installed(Vec<InstalledArtifact>),
}

/// The answer to request `id`, one input chunk: a path, the line's
/// artifacts, or why there is neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactAnswer {
    #[garde(skip)]
    pub id: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1)))]
    pub path: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(dive)]
    pub installed: Option<Vec<InstalledArtifact>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1)))]
    pub error: Option<String>,
}

impl ArtifactAnswer {
    pub fn new(id: u64, result: Result<ArtifactReply, String>) -> Self {
        let mut answer = Self {
            id,
            path: None,
            installed: None,
            error: None,
        };
        match result {
            Ok(ArtifactReply::Path(path)) => answer.path = Some(path),
            Ok(ArtifactReply::Installed(installed)) => answer.installed = Some(installed),
            Err(error) => answer.error = Some(error),
        }
        answer
    }

    /// The answer's outcome; one that carries more than one or none is
    /// invalid.
    pub fn outcome(&self) -> Result<Result<ArtifactReply, String>, ProtocolError> {
        garde::Validate::validate(self).map_err(ProtocolError::from)?;
        match (&self.path, &self.installed, &self.error) {
            (Some(path), None, None) => Ok(Ok(ArtifactReply::Path(path.clone()))),
            (None, Some(installed), None) => Ok(Ok(ArtifactReply::Installed(installed.clone()))),
            (None, None, Some(error)) => Ok(Err(error.clone())),
            _ => Err(ProtocolError::Invalid(
                "an artifact answer carries a path, the installed artifacts or an error".into(),
            )),
        }
    }
}
