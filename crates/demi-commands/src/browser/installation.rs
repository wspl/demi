//! Pinned Chrome for Testing archives, verified before atomic installation.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

use demi_command_service::protocol::host_target;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, sync::Mutex};
use tokio_util::sync::CancellationToken;

use demi_builtin_protocol::release::{BrowserInstallation, BrowserRelease, BrowserRuntimeConfig};

use super::{BrowserError, Result};

#[derive(Default)]
pub(super) struct Installation {
    installed: Mutex<Option<PathBuf>>,
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
    /// Install the release once per service; each browser retains its own profile.
    pub async fn executable(&self, cancel: &CancellationToken) -> Result<PathBuf> {
        let mut installed = tokio::select! {
            _ = cancel.cancelled() => return Err(BrowserError::Cancelled),
            lock = self.installed.lock() => lock,
        };
        if let Some(path) = &*installed {
            return Ok(path.clone());
        }
        let release = release()?;
        let record = release
            .platforms
            .into_iter()
            .find(|record| record.target == host_target())
            .ok_or_else(|| {
                BrowserError::Configuration(format!(
                    "Chrome for Testing {} is unavailable on {}",
                    release.version,
                    host_target()
                ))
            })?;
        #[cfg(unix)]
        if let Some(executable) = verify_installation(
            &PathBuf::from("/opt/demi/browsers").join(&record.sha256),
            &record.sha256,
            &record.executable,
            cancel,
        )
        .await?
        {
            *installed = Some(executable.clone());
            return Ok(executable);
        }
        let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        let config = BrowserRuntimeConfig::new(home)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        let home = PathBuf::from(config.home);
        if !home.is_absolute() {
            return Err(BrowserError::Configuration(
                "browser installation home must be absolute".into(),
            ));
        }
        let root = home.join(".demi/browsers");
        tokio::fs::create_dir_all(&root).await?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(format!("{}.lock", record.sha256)))?;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(BrowserError::Cancelled),
                        _ = tokio::time::sleep(Duration::from_millis(50)) => {},
                    }
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let destination = root.join(&record.sha256);
        let executable = destination.join(&record.executable);
        if verify_installation(&destination, &record.sha256, &record.executable, cancel)
            .await?
            .is_none()
        {
            let temporary = tempfile::Builder::new()
                .prefix("chrome-install-")
                .tempdir_in(&root)?;
            let archive_path = temporary.path().join("chrome.zip");
            let http = reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(300))
                .build()
                .map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(BrowserError::Cancelled),
                response = http.get(&record.url).send() => response.map_err(|error| BrowserError::Configuration(error.to_string()))?,
            }.error_for_status().map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let mut stream = response.bytes_stream();
            let mut output = tokio::fs::File::create(&archive_path).await?;
            let mut hash = Sha256::new();
            let mut size = 0_u64;
            loop {
                let chunk = tokio::select! {
                    _ = cancel.cancelled() => return Err(BrowserError::Cancelled),
                    chunk = stream.next() => chunk,
                };
                let Some(chunk) = chunk else {
                    break;
                };
                let chunk =
                    chunk.map_err(|error| BrowserError::Configuration(error.to_string()))?;
                size += chunk.len() as u64;
                if size > record.size {
                    return Err(BrowserError::Configuration(
                        "Chrome archive exceeds pinned size".into(),
                    ));
                }
                hash.update(&chunk);
                output.write_all(&chunk).await?;
            }
            output.flush().await?;
            drop(output);
            if size != record.size || format!("{:x}", hash.finalize()) != record.sha256 {
                return Err(BrowserError::Configuration(
                    "Chrome archive failed size or SHA-256 verification".into(),
                ));
            }
            let extraction = temporary.path().join("extracted");
            let extract_to = extraction.clone();
            let cancelled = cancel.clone();
            // Join extraction even on cancellation, so the temporary directory has one owner.
            tokio::task::spawn_blocking(move || {
                let reader = ChromeArchive {
                    file: File::open(archive_path)?,
                    cancelled,
                };
                let mut archive = zip::ZipArchive::new(reader).map_err(std::io::Error::other)?;
                archive.extract(extract_to).map_err(std::io::Error::other)
            })
            .await??;
            let digest = demi_artifact::digest(
                &extraction.join(&record.executable),
                1024 * 1024 * 1024,
                cancel,
            )
            .await
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let receipt = BrowserInstallation {
                archive_hash: record.sha256,
                executable_hash: digest.sha256,
            };
            tokio::fs::write(
                extraction.join("receipt.json"),
                serde_json::to_vec(&receipt)
                    .map_err(|error| BrowserError::Configuration(error.to_string()))?,
            )
            .await?;
            if cancel.is_cancelled() {
                return Err(BrowserError::Cancelled);
            }
            tokio::fs::rename(extraction, &destination).await?;
        }
        *installed = Some(executable.clone());
        Ok(executable)
    }
}

/// Validate a pinned Chrome installation before using either image or user storage.
async fn verify_installation(
    destination: &Path,
    archive_hash: &str,
    executable: &str,
    cancel: &CancellationToken,
) -> Result<Option<PathBuf>> {
    if !tokio::fs::try_exists(destination).await? {
        return Ok(None);
    }
    let receipt = BrowserInstallation::parse(
        &tokio::fs::read(destination.join("receipt.json")).await?,
    )
    .map_err(|error| {
        BrowserError::Configuration(format!("invalid Chrome installation receipt: {error}"))
    })?;
    let executable = destination.join(executable);
    let actual = demi_artifact::digest(&executable, 1024 * 1024 * 1024, cancel)
        .await
        .map_err(|error| BrowserError::Configuration(error.to_string()))?;
    if receipt.archive_hash != archive_hash || receipt.executable_hash != actual.sha256 {
        return Err(BrowserError::Configuration(
            "installed Chrome failed integrity verification".into(),
        ));
    }
    Ok(Some(executable))
}

/// zip has safe extraction and CRC checks but no cancellation API; stop its reads.
struct ChromeArchive {
    file: File,
    cancelled: CancellationToken,
}
impl Read for ChromeArchive {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancelled.is_cancelled() {
            return Err(std::io::Error::other("Chrome installation cancelled"));
        }
        self.file.read(buffer)
    }
}
impl Seek for ChromeArchive {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}
