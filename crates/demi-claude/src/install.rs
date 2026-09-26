//! Verified installations of the Claude Code CLI, one directory per version.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

use demi_artifact::{Digest, InstallLock};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use demi_claude_protocol::{
    Artifact, ErrorCode, Installed, Release, Status, is_version, parse_version,
};

use crate::{
    platform,
    release::{self, Transport},
};

/// The executable's file name inside a version directory.
const BINARY: &str = if cfg!(windows) {
    "claude.exe"
} else {
    "claude"
};

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
    /// The code the caller branches on; a cancellation has none, since it
    /// ends the invocation without a document.
    pub fn code(&self) -> Option<ErrorCode> {
        Some(match self {
            Self::InvalidRelease(_) => ErrorCode::InvalidRelease,
            Self::UnsupportedPlatform(_) => ErrorCode::UnsupportedPlatform,
            Self::DownloadFailed(_) => ErrorCode::DownloadFailed,
            Self::VerificationFailed(_) => ErrorCode::VerificationFailed,
            Self::InstallFailed(_) => ErrorCode::InstallFailed,
            Self::Cancelled => return None,
        })
    }
}

impl From<std::io::Error> for EnsureError {
    fn from(error: std::io::Error) -> Self {
        Self::InstallFailed(error.to_string())
    }
}

impl From<demi_artifact::Error> for EnsureError {
    fn from(error: demi_artifact::Error) -> Self {
        match error {
            demi_artifact::Error::Cancelled => Self::Cancelled,
            error => Self::InstallFailed(error.to_string()),
        }
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
        let home = std::env::home_dir()
            .filter(|home| !home.as_os_str().is_empty())
            .map(|home| home.join(".demi/claude"));
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

#[derive(Default)]
pub struct Installer {
    roots: Roots,
    transport: Transport,
    /// Executables whose full digest this process has checked, by path and
    /// SHA-256. A `std` mutex: each section only looks up or inserts, never
    /// awaiting.
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
        release::parse(input, self.transport)
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
                let Some(name) = entry.file_name().to_str().map(String::from) else {
                    continue;
                };
                // A directory whose name is not a version is not an installation.
                let Some(version) = parse_version(&name) else {
                    continue;
                };
                let path = entry.path().join(BINARY);
                let usable = read_receipt(&entry.path())
                    .await
                    .is_some_and(|receipt| receipt.version == name)
                    && tokio::fs::metadata(&path)
                        .await
                        .is_ok_and(|metadata| metadata.is_file());
                if usable {
                    installed.push((version, Installed { version: name, path }));
                }
            }
        }
        // Newest first, the image's before the user's for one version.
        installed.sort_by(|(left, left_installed), (right, right_installed)| {
            right.cmp(left).then_with(|| left_installed.path.cmp(&right_installed.path))
        });
        Ok(Status {
            platform: platform.into(),
            installed: installed.into_iter().map(|(_, installed)| installed).collect(),
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
        let _lock = InstallLock::acquire(&root.join(format!("{}.lock", expected.version)), cancel).await?;
        let destination = root.join(&expected.version);
        if let Some(path) = self.usable(&destination, expected, cancel).await? {
            return Ok(path);
        }
        let temporary = tempfile::Builder::new()
            .prefix("install-")
            .tempdir_in(root)?;
        let staged = temporary.path().join(&expected.version);
        tokio::fs::create_dir(&staged).await?;
        self.download(artifact, expected, &staged.join(BINARY), cancel)
            .await?;
        demi_artifact::receipt::write(&staged, expected).await?;
        if cancel.is_cancelled() {
            return Err(EnsureError::Cancelled);
        }
        demi_artifact::publish_directory(&staged, &destination).await?;
        let path = destination.join(BINARY);
        self.verified()
            .insert((path.clone(), expected.sha256.clone()));
        Ok(path)
    }

    /// Stream the artifact into `path`, enforcing its size and SHA-256, and
    /// make it executable. The caller owns the directory and removes it on
    /// any failure.
    async fn download(
        &self,
        artifact: &Artifact,
        expected: &Receipt,
        path: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), EnsureError> {
        let version = &expected.version;
        let host = release::host(artifact);
        let client = match self.transport {
            Transport::Https => demi_artifact::client(),
            #[cfg(test)]
            Transport::HttpsOrLoopbackHttp => demi_artifact::client_allowing_http(),
        }
        .map_err(|error| EnsureError::DownloadFailed(error.to_string()))?;
        let declared = Digest {
            size: expected.size,
            sha256: expected.sha256.clone(),
        };
        let mut output = tokio::fs::File::create(path).await?;
        let downloaded =
            demi_artifact::download(&client, &artifact.url, &declared, &mut output, cancel).await;
        match downloaded {
            Ok(()) => {}
            Err(demi_artifact::Error::Cancelled) => return Err(EnsureError::Cancelled),
            Err(error @ (demi_artifact::Error::Download(_) | demi_artifact::Error::Rejected { .. })) => {
                return Err(EnsureError::DownloadFailed(format!(
                    "Claude Code {version} download from {host} failed: {error}"
                )));
            }
            Err(
                error @ (demi_artifact::Error::TooLarge { .. }
                | demi_artifact::Error::Size { .. }
                | demi_artifact::Error::Digest),
            ) => {
                return Err(EnsureError::VerificationFailed(format!(
                    "Claude Code {version} from {host}: {error}"
                )));
            }
            Err(error) => return Err(error.into()),
        }
        output.sync_all().await?;
        drop(output);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await?;
        }
        Ok(())
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
        match demi_artifact::digest(&key.0, expected.size, cancel).await {
            Ok(digest) if digest.sha256 == expected.sha256 => {
                self.verified().insert(key.clone());
                Ok(Some(key.0))
            }
            Err(demi_artifact::Error::Cancelled) => Err(EnsureError::Cancelled),
            _ => Ok(None),
        }
    }

    fn verified(&self) -> std::sync::MutexGuard<'_, HashSet<(PathBuf, String)>> {
        self.verified
            .lock()
            .expect("a panic interrupted an update of the verified set")
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

/// The receipt in `directory`; none when it has none or one that does not
/// decode, which makes the installation unusable.
async fn read_receipt(directory: &Path) -> Option<Receipt> {
    let bytes = demi_artifact::receipt::read(directory).await.ok()??;
    serde_json::from_slice(&bytes).ok()
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
            .is_some_and(|name| name != keep && is_version(name));
        if other && entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
            let _unremovable = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}
