//! Verified installations of the Claude Code CLI, one directory per version.

use std::{
    collections::HashSet,
    fs::File,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::Duration,
};

use demi_command_service::{ServiceError, integrity::artifact_digest};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::{
    platform,
    release::{Artifact, Release, Transport},
    version,
};

/// The executable's file name inside a version directory.
const BINARY: &str = if cfg!(windows) {
    "claude.exe"
} else {
    "claude"
};
const RECEIPT: &str = "receipt.json";

/// Why an operation failed. `code` is what the caller branches on; a
/// cancellation ends the invocation without an answer.
#[derive(Debug, thiserror::Error)]
pub enum EnsureError {
    #[error("invalid release record: {0}")]
    InvalidRelease(String),
    #[error("{0}")]
    UnsupportedPlatform(String),
    #[error("{0}")]
    DownloadFailed(String),
    #[error("{0}")]
    VerificationFailed(String),
    #[error("Claude Code installation failed: {0}")]
    InstallFailed(String),
    #[error("cancelled")]
    Cancelled,
}

impl EnsureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRelease(_) => "invalid_release",
            Self::UnsupportedPlatform(_) => "unsupported_platform",
            Self::DownloadFailed(_) => "download_failed",
            Self::VerificationFailed(_) => "verification_failed",
            Self::InstallFailed(_) => "install_failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl From<std::io::Error> for EnsureError {
    fn from(error: std::io::Error) -> Self {
        Self::InstallFailed(error.to_string())
    }
}

/// Where installations live.
#[derive(Clone, Debug)]
pub struct Roots {
    /// The user's root, `~/.demi/claude`: installed into, and cleaned of other versions.
    pub home: Option<PathBuf>,
    /// An image's preinstalled root: used when it has the wanted version, never written.
    pub image: Option<PathBuf>,
}

impl Default for Roots {
    fn default() -> Self {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join(".demi/claude"));
        let image = cfg!(unix).then(|| PathBuf::from("/opt/demi/claude"));
        Self { home, image }
    }
}

/// What a version directory records about the executable beside it.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: String,
    platform: String,
    sha256: String,
    size: u64,
}

/// One usable executable.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Installed {
    pub version: String,
    pub path: PathBuf,
}

/// This machine's platform key and its installations, newest version first.
#[derive(Debug, Serialize)]
pub struct Status {
    pub platform: String,
    pub installed: Vec<Installed>,
}

#[derive(Default)]
pub struct Installer {
    roots: Roots,
    transport: Transport,
    /// Executables whose full digest this process has checked, by path and SHA-256.
    verified: Mutex<HashSet<(PathBuf, String)>>,
}

impl Installer {
    pub fn new(roots: Roots) -> Self {
        Self {
            roots,
            ..Self::default()
        }
    }

    /// An installer that also downloads from a fixture server on `127.0.0.1`.
    #[cfg(test)]
    pub(crate) fn with_loopback_http(roots: Roots) -> Self {
        Self {
            roots,
            transport: Transport::HttpsOrLoopbackHttp,
            verified: Mutex::default(),
        }
    }

    /// Parse and validate the input of `claude.ensure`.
    pub fn release(&self, input: &[u8]) -> Result<Release, EnsureError> {
        Release::parse(input, self.transport)
    }

    /// Answer the release's executable for this machine, installing it when no
    /// usable installation exists. Callers of one version, in this process and
    /// others, share one download. Every other version under the user's root is
    /// removed afterwards.
    pub async fn ensure(
        &self,
        release: &Release,
        cancel: &CancellationToken,
    ) -> Result<Installed, EnsureError> {
        let platform = current_platform()?;
        let artifact = release.platforms.get(platform).ok_or_else(|| {
            EnsureError::UnsupportedPlatform(format!(
                "Claude Code {} has no build for {platform}",
                release.version
            ))
        })?;
        let expected = Receipt {
            version: release.version.clone(),
            platform: platform.into(),
            sha256: artifact.sha256.clone(),
            size: artifact.size,
        };
        let path = match self.preinstalled(&expected, cancel).await? {
            Some(path) => path,
            None => self.install(artifact, &expected, cancel).await?,
        };
        if let Ok(home) = self.home() {
            remove_other_versions(home, &release.version).await;
        }
        Ok(Installed {
            version: expected.version,
            path,
        })
    }

    /// The installations under both roots that have a receipt and an executable.
    /// Nothing is hashed.
    pub async fn status(&self) -> Result<Status, EnsureError> {
        let platform = current_platform()?;
        let mut installed = Vec::new();
        for root in [self.roots.image.as_deref(), self.home().ok()]
            .into_iter()
            .flatten()
        {
            let Ok(mut entries) = tokio::fs::read_dir(root).await else {
                continue;
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let Some(version) = entry.file_name().to_str().map(String::from) else {
                    continue;
                };
                let path = entry.path().join(BINARY);
                let usable = version::is_valid(&version)
                    && read_receipt(&entry.path())
                        .await
                        .is_some_and(|receipt| receipt.version == version)
                    && tokio::fs::metadata(&path)
                        .await
                        .is_ok_and(|metadata| metadata.is_file());
                if usable {
                    installed.push(Installed { version, path });
                }
            }
        }
        installed.sort_by(|left, right| {
            version::compare(&right.version, &left.version).then_with(|| left.path.cmp(&right.path))
        });
        Ok(Status {
            platform: platform.into(),
            installed,
        })
    }

    fn home(&self) -> Result<&Path, EnsureError> {
        let home = self.roots.home.as_deref().ok_or_else(|| {
            EnsureError::InstallFailed("the user's home directory is not set".into())
        })?;
        if !home.is_absolute() {
            return Err(EnsureError::InstallFailed(
                "the user's home directory must be absolute".into(),
            ));
        }
        Ok(home)
    }

    async fn preinstalled(
        &self,
        expected: &Receipt,
        cancel: &CancellationToken,
    ) -> Result<Option<PathBuf>, EnsureError> {
        match &self.roots.image {
            Some(image) => {
                self.usable(&image.join(&expected.version), expected, cancel)
                    .await
            }
            None => Ok(None),
        }
    }

    /// Verify or create `<home>/<version>` while holding `<home>/<version>.lock`.
    async fn install(
        &self,
        artifact: &Artifact,
        expected: &Receipt,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, EnsureError> {
        let root = self.home()?;
        tokio::fs::create_dir_all(root).await?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(format!("{}.lock", expected.version)))?;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(EnsureError::Cancelled),
                        _ = tokio::time::sleep(Duration::from_millis(50)) => {},
                    }
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let destination = root.join(&expected.version);
        if let Some(path) = self.usable(&destination, expected, cancel).await? {
            return Ok(path);
        }
        let temporary = tempfile::Builder::new()
            .prefix("install-")
            .tempdir_in(root)?;
        let staged = temporary.path().join(&expected.version);
        tokio::fs::create_dir(&staged).await?;
        download(
            artifact,
            expected,
            &staged.join(BINARY),
            self.transport,
            cancel,
        )
        .await?;
        let receipt = serde_json::to_vec(expected)
            .map_err(|error| EnsureError::InstallFailed(error.to_string()))?;
        tokio::fs::write(staged.join(RECEIPT), receipt).await?;
        if cancel.is_cancelled() {
            return Err(EnsureError::Cancelled);
        }
        match tokio::fs::remove_dir_all(&destination).await {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        tokio::fs::rename(&staged, &destination).await?;
        let path = destination.join(BINARY);
        self.verified()
            .insert((path.clone(), expected.sha256.clone()));
        Ok(path)
    }

    /// The executable in `directory` when its receipt equals `expected`, its
    /// size matches and its SHA-256 matches. The digest is computed once per
    /// process for each executable; later calls compare receipt and size only.
    async fn usable(
        &self,
        directory: &Path,
        expected: &Receipt,
        cancel: &CancellationToken,
    ) -> Result<Option<PathBuf>, EnsureError> {
        if read_receipt(directory).await.as_ref() != Some(expected) {
            return Ok(None);
        }
        let path = directory.join(BINARY);
        let sized = tokio::fs::metadata(&path)
            .await
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == expected.size);
        if !sized {
            return Ok(None);
        }
        let key = (path, expected.sha256.clone());
        if self.verified().contains(&key) {
            return Ok(Some(key.0));
        }
        match artifact_digest(&key.0, expected.size, cancel).await {
            Ok(digest) if digest.sha256 == expected.sha256 => {
                self.verified().insert(key.clone());
                Ok(Some(key.0))
            }
            Err(ServiceError::Cancelled) => Err(EnsureError::Cancelled),
            _ => Ok(None),
        }
    }

    fn verified(&self) -> std::sync::MutexGuard<'_, HashSet<(PathBuf, String)>> {
        self.verified.lock().unwrap_or_else(PoisonError::into_inner)
    }
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

async fn read_receipt(directory: &Path) -> Option<Receipt> {
    let bytes = tokio::fs::read(directory.join(RECEIPT)).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Stream the artifact into `path`, enforcing its size and SHA-256, and make it
/// executable. The caller owns the directory and removes it on any failure.
async fn download(
    artifact: &Artifact,
    expected: &Receipt,
    path: &Path,
    transport: Transport,
    cancel: &CancellationToken,
) -> Result<(), EnsureError> {
    let version = &expected.version;
    let host = artifact.host();
    let failed = |error: reqwest::Error| {
        EnsureError::DownloadFailed(format!(
            "Claude Code {version} download from {host} failed: {}",
            error.without_url()
        ))
    };
    let http = reqwest::Client::builder()
        .https_only(transport == Transport::Https)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(failed)?;
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(EnsureError::Cancelled),
        response = http.get(&artifact.url).send() => response.map_err(failed)?,
    };
    if !response.status().is_success() {
        return Err(EnsureError::DownloadFailed(format!(
            "Claude Code {version} download from {host} failed: HTTP status {}",
            response.status().as_u16()
        )));
    }
    let mut stream = response.bytes_stream();
    let mut output = tokio::fs::File::create(path).await?;
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(EnsureError::Cancelled),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(failed)?;
        size += chunk.len() as u64;
        if size > expected.size {
            return Err(EnsureError::VerificationFailed(format!(
                "Claude Code {version} from {host} exceeds its declared size of {} bytes",
                expected.size
            )));
        }
        hash.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    if size != expected.size {
        return Err(EnsureError::VerificationFailed(format!(
            "Claude Code {version} from {host} has {size} bytes, not the declared {}",
            expected.size
        )));
    }
    if format!("{:x}", hash.finalize()) != expected.sha256 {
        return Err(EnsureError::VerificationFailed(format!(
            "Claude Code {version} from {host} does not match its declared SHA-256"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await?;
    }
    Ok(())
}

/// Remove every other version directory. A failure leaves that version for a
/// later call: Windows cannot remove an executable that is running.
async fn remove_other_versions(root: &Path, keep: &str) {
    let Ok(mut entries) = tokio::fs::read_dir(root).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let other = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name != keep && version::is_valid(name));
        if other && entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
            let _unremovable = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}
