//! The pinned Chrome for Testing release, installed once per service and
//! verified before use (`browser.md` § Browser distribution) by `artifact`'s
//! archive installation, the one the Cloud image build uses too.

use std::path::PathBuf;

use demi_artifact::{Archive, Digest};
use demi_builtin_protocol::release::BrowserRelease;
use demi_command_service::protocol::host_target;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use super::{BrowserError, Result};

/// Where the Cloud image preinstalls the release.
#[cfg(unix)]
const IMAGE_ROOT: &str = "/opt/demi/browsers";
/// Where the service installs it, under the user's home.
const HOME_ROOT: &str = ".demi/browsers";

#[derive(Default)]
pub(super) struct Installation {
    /// The verified executable, once a caller installed or found it.
    installed: OnceCell<PathBuf>,
}

/// The pinned Chrome for Testing release (`browser.md` § Browser distribution).
fn release() -> Result<BrowserRelease> {
    BrowserRelease::pinned().map_err(|error| BrowserError::Configuration(error.to_string()))
}

/// The pinned release's version, such as `153.0.8010.36`.
pub fn pinned_version() -> Result<String> {
    Ok(release()?.version)
}

impl Installation {
    /// The release's verified executable, installed on first use. Callers
    /// that arrive meanwhile wait for it and stop waiting when cancelled; a
    /// cancelled install lets the next caller try again. The install's own
    /// work watches the same token, so dropping it on cancellation leaves
    /// nothing running.
    pub async fn executable(&self, cancel: &CancellationToken) -> Result<PathBuf> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(BrowserError::Cancelled),
            result = self.installed.get_or_try_init(|| install(cancel)) => result.cloned(),
        }
    }
}

/// Finds the release preinstalled in the image, or installs it under the
/// user's home, where another service installing it at the same time finds
/// this one's result. An installation that fails its check fails the
/// install: nothing falls back to another location.
async fn install(cancel: &CancellationToken) -> Result<PathBuf> {
    let release = release()?;
    let version = release.version;
    let record = release
        .platforms
        .into_iter()
        .find(|record| record.target == host_target())
        .ok_or_else(|| {
            BrowserError::Installation(format!("{version} is unavailable on {}", host_target()))
        })?;
    let archive = Archive {
        url: record.url,
        digest: Digest {
            size: record.size,
            sha256: record.sha256,
        },
        executable: record.executable,
    };
    #[cfg(unix)]
    {
        let preinstalled = PathBuf::from(IMAGE_ROOT).join(&archive.digest.sha256);
        let found = demi_artifact::installed(&preinstalled, &archive, cancel)
            .await
            .map_err(|error| failed(&version, error))?;
        if let Some(executable) = found {
            return Ok(executable);
        }
    }
    let home = std::env::home_dir()
        .filter(|home| home.is_absolute())
        .ok_or_else(|| {
            BrowserError::Installation(format!(
                "{version} needs an absolute home directory to install into"
            ))
        })?;
    let client = demi_artifact::client().map_err(|error| failed(&version, error))?;
    demi_artifact::install_archive(&client, &home.join(HOME_ROOT), &archive, cancel)
        .await
        .map_err(|error| failed(&version, error))
}

/// Where any release's Chrome executables live, for finding its processes.
pub(super) fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(unix)]
    roots.push(PathBuf::from(IMAGE_ROOT));
    if let Some(home) = std::env::home_dir().filter(|home| home.is_absolute()) {
        roots.push(home.join(HOME_ROOT));
    }
    roots
}

/// What a failure of the verified-bytes library means for the install.
fn failed(version: &str, error: demi_artifact::Error) -> BrowserError {
    use demi_artifact::Error;
    match error {
        Error::Cancelled => BrowserError::Cancelled,
        Error::Io(error) => BrowserError::Io(error),
        error @ (Error::Download(_) | Error::Rejected { .. }) => {
            BrowserError::Installation(format!("{version} download failed: {error}"))
        }
        error @ (Error::TooLarge { .. } | Error::Size { .. } | Error::Digest) => {
            BrowserError::Installation(format!("{version} failed verification: {error}"))
        }
        error @ (Error::Archive(_) | Error::Installation { .. }) => {
            BrowserError::Installation(format!("{version}: {error}"))
        }
        // An installation publishes no release, so it meets no conflict.
        error @ Error::Conflict(_) => {
            BrowserError::Installation(format!("{version} installation failed: {error}"))
        }
    }
}
