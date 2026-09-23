//! The `demi.claude` package's contract (`claude-code.md` § The package):
//! `claude.ensure` installs the Claude Code CLI a release record names and
//! `claude.status` lists the installations. Each writes one JSON document to
//! stdout, a failure included, because a service stream carries only stdout.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;

/// The package's operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// `claude.ensure`: installs the version a release record names.
    Ensure,
    /// `claude.status`: lists the installed versions.
    Status,
}

impl Operation {
    /// Every operation, in the order the descriptor lists them.
    pub const ALL: [Operation; 2] = [Self::Ensure, Self::Status];

    /// The operation's name, such as `claude.ensure`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ensure => "claude.ensure",
            Self::Status => "claude.status",
        }
    }

    /// The operation named `name`, if the package has it.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|operation| operation.name() == name)
    }
}

/// Why a record was refused.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error(transparent)]
    Shape(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

impl From<garde::Report> for DecodeError {
    fn from(report: garde::Report) -> Self {
        Self::Invalid(report.to_string().trim_end().to_owned())
    }
}

/// `version` as a SemVer version when it is one without build metadata, such
/// as `2.1.3` or `2.1.3-beta.1`. A version names its installation directory,
/// so `+build` is refused.
pub fn parse_version(version: &str) -> Option<semver::Version> {
    semver::Version::parse(version)
        .ok()
        .filter(|version| version.build.is_empty())
}

/// Whether [`parse_version`] accepts `version`.
pub fn is_version(version: &str) -> bool {
    parse_version(version).is_some()
}

fn cli_version(value: &str, _: &()) -> garde::Result {
    if !is_version(value) {
        return Err(garde::Error::new(
            "is not a SemVer version without build metadata",
        ));
    }
    Ok(())
}

/// What `claude.ensure` reads from stdin: one CLI version and its official
/// executable for each platform key. Every entry is checked, not only this
/// machine's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct Release {
    #[garde(custom(cli_version))]
    pub version: String,
    #[garde(dive)]
    pub platforms: BTreeMap<String, Artifact>,
}

impl Release {
    /// Decodes a release record.
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        let release: Self = serde_json::from_slice(bytes)?;
        garde::Validate::validate(&release)?;
        Ok(release)
    }
}

/// Where one platform's executable is, its byte size and its SHA-256 in
/// lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    #[garde(url)]
    pub url: String,
    #[garde(range(min = 1))]
    pub size: u64,
    #[garde(pattern(r"^[a-f0-9]{64}$"))]
    pub sha256: String,
}

/// One usable executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    pub version: String,
    pub path: PathBuf,
}

/// What `claude.status` answers: this machine's platform key and its
/// installations, newest version first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub platform: String,
    pub installed: Vec<Installed>,
}

/// Why an operation failed; a cancelled invocation writes no document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRelease,
    UnsupportedPlatform,
    DownloadFailed,
    VerificationFailed,
    InstallFailed,
}

serde_plain::derive_display_from_serialize!(ErrorCode);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub code: ErrorCode,
    pub message: String,
}

/// The document an operation writes: `{"ok": true, ...answer}`, or
/// `{"ok": false, "code", "message"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply<T> {
    Done(T),
    Failed(Failure),
}

impl<T: Serialize> Serialize for Reply<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Done<'a, T> {
            ok: bool,
            #[serde(flatten)]
            answer: &'a T,
        }
        #[derive(Serialize)]
        struct Failed<'a> {
            ok: bool,
            #[serde(flatten)]
            failure: &'a Failure,
        }
        match self {
            Self::Done(answer) => Done { ok: true, answer }.serialize(serializer),
            Self::Failed(failure) => Failed { ok: false, failure }.serialize(serializer),
        }
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Reply<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let mut document = serde_json::Map::<String, Value>::deserialize(deserializer)?;
        let rest = |document| Value::Object(document);
        match document.remove("ok") {
            Some(Value::Bool(true)) => T::deserialize(rest(document)).map(Self::Done).map_err(D::Error::custom),
            Some(Value::Bool(false)) => Failure::deserialize(rest(document))
                .map(Self::Failed)
                .map_err(D::Error::custom),
            Some(_) => Err(D::Error::custom("`ok` is not a boolean")),
            None => Err(D::Error::missing_field("ok")),
        }
    }
}
