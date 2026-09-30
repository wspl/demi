//! The pinned Chrome for Testing release, installed once per service and
//! verified before use (`browser.md` § Browser distribution) by `artifact`'s
//! archive installation, the one the Cloud image build uses too.

use std::path::PathBuf;

use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use demi_artifact::{Archive, Digest};
use demi_browser_protocol::release::BrowserRelease;
use demi_command_service::protocol::host_target;

use crate::operation::{BrowserError, Result};

/// Where the service installs it, under the user's home.
const HOME_ROOT: &str = ".demi/browsers";

/// Where a service finds the pinned release, each holding it in the
/// directory its archive's SHA-256 names (`browser.md` § Browser
/// distribution).
#[derive(Debug, Clone)]
pub struct BrowserDirectories {
    /// Where an image preinstalls it; looked at first.
    pub image: Option<PathBuf>,
    /// Where the service installs it otherwise; without one, it cannot.
    pub install: Option<PathBuf>,
}

impl BrowserDirectories {
    /// The Host's: the Cloud image's `/opt/demi/browsers` on Unix, then
    /// `.demi/browsers` in the user's home, when the home is absolute.
    pub fn host() -> Self {
        Self {
            image: cfg!(unix)
                .then(|| PathBuf::from(demi_browser_protocol::release::IMAGE_BROWSERS)),
            install: std::env::home_dir()
                .filter(|home| home.is_absolute())
                .map(|home| home.join(HOME_ROOT)),
        }
    }

    /// Where any release's Chrome executables live, for finding its processes.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.image.iter().chain(&self.install).cloned().collect()
    }
}

pub struct Installation {
    directories: BrowserDirectories,
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

/// The pinned release's archive for this Host's platform.
pub fn pinned_archive() -> Result<Archive> {
    let release = release()?;
    let version = release.version;
    let record = release
        .platforms
        .into_iter()
        .find(|record| record.target == host_target())
        .ok_or_else(|| {
            BrowserError::Installation(format!("{version} is unavailable on {}", host_target()))
        })?;
    Ok(Archive {
        url: record.url,
        digest: Digest {
            size: record.size,
            sha256: record.sha256,
        },
        executable: record.executable,
    })
}

impl Installation {
    /// The release in `directories`, found or installed on first use.
    pub fn new(directories: BrowserDirectories) -> Self {
        Self {
            directories,
            installed: OnceCell::new(),
        }
    }

    /// The release's verified executable, installed on first use. Callers
    /// that arrive meanwhile wait for it and stop waiting when cancelled; a
    /// cancelled install lets the next caller try again. The install's own
    /// work watches the same token, so dropping it on cancellation leaves
    /// nothing running.
    pub async fn executable(&self, cancel: &CancellationToken) -> Result<PathBuf> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(BrowserError::Cancelled),
            result = self.installed.get_or_try_init(|| install(&self.directories, cancel)) => result.cloned(),
        }
    }
}

/// Finds the release preinstalled in the image's directory, or installs it
/// in the install directory, where another service installing it at the
/// same time finds this one's result. An installation that fails its check
/// fails the install: nothing falls back to another location.
async fn install(directories: &BrowserDirectories, cancel: &CancellationToken) -> Result<PathBuf> {
    let version = pinned_version()?;
    let archive = pinned_archive()?;
    if let Some(image) = &directories.image {
        let found = demi_artifact::installed(&image.join(&archive.digest.sha256), &archive, cancel)
            .await
            .map_err(|error| failed(&version, error))?;
        if let Some(executable) = found {
            return Ok(executable);
        }
    }
    let root = directories.install.as_ref().ok_or_else(|| {
        BrowserError::Installation(format!(
            "{version} needs an absolute home directory to install into"
        ))
    })?;
    let client = demi_artifact::client().map_err(|error| failed(&version, error))?;
    demi_artifact::install_archive(&client, root, &archive, cancel)
        .await
        .map_err(|error| failed(&version, error))
}

/// What a failure of the verified-bytes library means for the install.
fn failed(version: &str, error: demi_artifact::Error) -> BrowserError {
    use demi_artifact::Error;
    match error {
        Error::Cancelled => BrowserError::Cancelled,
        Error::Io(error) => BrowserError::Io(error),
        error @ (Error::Download(_) | Error::Rejected { .. } | Error::Coding(_)) => {
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
