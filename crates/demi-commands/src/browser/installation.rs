//! The pinned Chrome for Testing release, installed once per service and
//! verified before use (`browser.md` § Browser distribution).

use std::path::{Path, PathBuf};

use demi_artifact::{Digest, InstallLock};
use demi_builtin_protocol::release::{BrowserInstallation, BrowserRelease, ReleasePlatform};
use demi_command_service::protocol::host_target;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use super::{BrowserError, Result};

/// Where the Cloud image preinstalls the release.
#[cfg(unix)]
const IMAGE_ROOT: &str = "/opt/demi/browsers";
/// The most bytes an installed executable may have, for its digest.
const EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Installation {
    /// The verified executable, once a caller installed or found it.
    installed: OnceCell<PathBuf>,
}

/// The pinned Chrome for Testing release (`browser.md` § Browser distribution).
fn release() -> Result<BrowserRelease> {
    BrowserRelease::parse(include_str!("releases/chrome.json"))
        .map_err(|error| BrowserError::Configuration(error.to_string()))
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

/// Finds the release preinstalled in the image or under the user's home, or
/// installs it under the home while holding the release's install lock, so
/// another service installing it at the same time finds this one's result.
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
    #[cfg(unix)]
    if let Some(executable) =
        verified(&Path::new(IMAGE_ROOT).join(&record.sha256), &record, &version, cancel).await?
    {
        return Ok(executable);
    }
    let home = std::env::home_dir()
        .filter(|home| home.is_absolute())
        .ok_or_else(|| {
            BrowserError::Installation(format!(
                "{version} needs an absolute home directory to install into"
            ))
        })?;
    let root = home.join(".demi/browsers");
    tokio::fs::create_dir_all(&root).await?;
    let _lock = InstallLock::acquire(&root.join(format!("{}.lock", record.sha256)), cancel)
        .await
        .map_err(|error| failed(&version, error))?;
    let destination = root.join(&record.sha256);
    if let Some(executable) = verified(&destination, &record, &version, cancel).await? {
        return Ok(executable);
    }
    let temporary = tempfile::Builder::new()
        .prefix("chrome-install-")
        .tempdir_in(&root)?;
    let archive = temporary.path().join("chrome.zip");
    let client = demi_artifact::client().map_err(|error| failed(&version, error))?;
    let mut output = tokio::fs::File::create(&archive).await?;
    demi_artifact::download(
        &client,
        &record.url,
        &Digest {
            size: record.size,
            sha256: record.sha256.clone(),
        },
        &mut output,
        cancel,
    )
    .await
    .map_err(|error| failed(&version, error))?;
    drop(output);
    let extraction = temporary.path().join("extracted");
    demi_artifact::extract_zip(&archive, &extraction, cancel)
        .await
        .map_err(|error| failed(&version, error))?;
    let executable = demi_artifact::digest(
        &extraction.join(&record.executable),
        EXECUTABLE_BYTES,
        cancel,
    )
    .await
    .map_err(|error| failed(&version, error))?;
    let receipt = BrowserInstallation {
        archive_hash: record.sha256.clone(),
        executable_hash: executable.sha256,
    };
    demi_artifact::receipt::write(&extraction, &receipt)
        .await
        .map_err(|error| failed(&version, error))?;
    if cancel.is_cancelled() {
        return Err(BrowserError::Cancelled);
    }
    demi_artifact::publish_directory(&extraction, &destination)
        .await
        .map_err(|error| failed(&version, error))?;
    Ok(destination.join(&record.executable))
}

/// The installation at `destination`, checked against its receipt; none
/// when nothing is installed there. One that fails the check fails the
/// install: nothing falls back to another location.
async fn verified(
    destination: &Path,
    record: &ReleasePlatform,
    version: &str,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>> {
    if !tokio::fs::try_exists(destination).await? {
        return Ok(None);
    }
    let receipt = demi_artifact::receipt::read(destination)
        .await
        .map_err(|error| failed(version, error))?
        .ok_or_else(|| BrowserError::Installation(format!("{version} installation has no receipt")))?;
    let receipt = BrowserInstallation::parse(&receipt).map_err(|error| {
        BrowserError::Installation(format!("{version} installation has an invalid receipt: {error}"))
    })?;
    let executable = destination.join(&record.executable);
    let actual = demi_artifact::digest(&executable, EXECUTABLE_BYTES, cancel)
        .await
        .map_err(|error| failed(version, error))?;
    if receipt.archive_hash != record.sha256 || receipt.executable_hash != actual.sha256 {
        return Err(BrowserError::Installation(format!(
            "{version} installation failed its integrity check"
        )));
    }
    Ok(Some(executable))
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
        error @ Error::Archive(_) => {
            BrowserError::Installation(format!("{version} could not be extracted: {error}"))
        }
    }
}
