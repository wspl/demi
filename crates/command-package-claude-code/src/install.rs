//! The Claude Code CLI on the machine that runs it (`claude-code.md` § The
//! package): the release's executable for this machine, which the runner
//! installs as the package's artifact `Claude Code`, and the versions of it
//! the runner has. Nothing here downloads or looks for the CLI.

use demi_command_package_claude_code_protocol::{ErrorCode, Installed, Release, Status};
use demi_command_protocol::{ArtifactForm, ArtifactInstall};
use demi_command_sdk::{Artifacts, ServiceError};

use crate::{platform, release};

/// The artifact line of the CLI.
pub const NAME: &str = "Claude Code";

/// Why an operation failed. `code` is what the caller branches on; a
/// cancellation ends the invocation without an answer.
#[derive(Debug, thiserror::Error)]
pub enum EnsureError {
    #[error("invalid release record: {0}")]
    InvalidRelease(String),
    #[error("{0}")]
    UnsupportedPlatform(String),
    #[error("Claude Code installation failed: {0}")]
    InstallFailed(String),
    #[error("cancelled")]
    Cancelled,
}

impl EnsureError {
    /// The code the caller branches on; a cancellation has none, since it
    /// ends the invocation without a document.
    pub fn code(&self) -> Option<ErrorCode> {
        Some(match self {
            Self::InvalidRelease(_) => ErrorCode::InvalidRelease,
            Self::UnsupportedPlatform(_) => ErrorCode::UnsupportedPlatform,
            Self::InstallFailed(_) => ErrorCode::InstallFailed,
            Self::Cancelled => return None,
        })
    }
}

impl From<ServiceError> for EnsureError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::Cancelled => Self::Cancelled,
            ServiceError::Artifacts(reason) => Self::InstallFailed(reason),
            error => Self::InstallFailed(error.to_string()),
        }
    }
}

/// Parses and validates the input of `claude-code.ensure`.
pub fn release(input: &[u8]) -> Result<Release, EnsureError> {
    release::parse(input)
}

/// The release's executable for this machine, which the runner installs for
/// `invocation` when it has none.
pub async fn ensure(
    artifacts: &Artifacts,
    invocation: &str,
    release: &Release,
) -> Result<Installed, EnsureError> {
    let platform = current_platform()?;
    let artifact = release.platforms.get(platform).ok_or_else(|| {
        EnsureError::UnsupportedPlatform(format!(
            "Claude Code {} has no build for {platform}",
            release.version
        ))
    })?;
    let install = ArtifactInstall {
        invocation: invocation.to_owned(),
        name: NAME.to_owned(),
        version: release.version.clone(),
        sha256: artifact.sha256.clone(),
        size: artifact.size,
        form: ArtifactForm::File,
        url: None,
    };
    let path = artifacts.install(install).await?;
    Ok(Installed {
        version: release.version.clone(),
        path,
    })
}

/// This machine's platform key, and the versions of the CLI the runner has,
/// the newest install first.
pub async fn status(artifacts: &Artifacts) -> Result<Status, EnsureError> {
    let platform = current_platform()?;
    let installed = artifacts
        .installed(NAME)
        .await?
        .into_iter()
        .map(|installed| Installed {
            version: installed.version,
            path: installed.path.into(),
        })
        .collect();
    Ok(Status {
        platform: platform.into(),
        installed,
    })
}

fn current_platform() -> Result<&'static str, EnsureError> {
    platform::current().ok_or_else(|| {
        EnsureError::UnsupportedPlatform(format!(
            "Claude Code has no build for this machine ({} {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })
}
