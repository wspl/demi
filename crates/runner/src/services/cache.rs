//! The verified executable cache (`native-runtime.md` § Install the selected
//! executable): one file per artifact digest, published only once its size
//! and SHA-256 match what the descriptor declares, and reused unread from
//! then on. Before it downloads an executable, it takes the copy the Host's
//! image preinstalled (§ Preinstalled executables), which it checks the same
//! way the first time the process needs it. The service registry starts one
//! install per digest at a time, so concurrent callers share it.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime},
};

use demi_artifact::{Digest, Mode, Permissions, Publication, Staged};
use demi_command_service::protocol::PackageArtifact;
use tokio_util::sync::CancellationToken;

use super::{ArtifactResolver, ArtifactSource, RuntimeError};

/// How long one attempt at an executable may take once it has its files:
/// the check of the image's copy, then the download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// What this process found when it checked the image's copy of each digest:
/// the executable, or none when the copy failed its check.
type Checked = HashMap<String, Option<PathBuf>>;

pub struct ArtifactCache {
    root: PathBuf,
    /// Where the Host's image preinstalls command executables, each alone in
    /// a directory named by its SHA-256. Nothing is there on a paired device.
    image: Option<PathBuf>,
    checked: Mutex<Checked>,
    http: reqwest::Client,
}

impl ArtifactCache {
    /// A cache in `root` that takes the executables preinstalled in `image`,
    /// when given, before it downloads one.
    pub async fn new(root: PathBuf, image: Option<PathBuf>) -> Result<Self, RuntimeError> {
        tokio::fs::create_dir_all(&root).await?;
        crate::fs::chmod(&root, 0o700).await?;
        Ok(Self {
            root,
            image,
            checked: Mutex::default(),
            // The pinned descriptor decides what is installed, so a backend
            // on plain HTTP may serve its executables itself.
            http: demi_artifact::client_allowing_http()?,
        })
    }

    /// The executable for `artifact`: the cached one, else the image's copy
    /// that matches it, else one downloaded into the cache. A cached entry
    /// that is not a regular file of the declared size fails the install.
    pub async fn install(
        &self,
        artifact: &PackageArtifact,
        resolver: &dyn ArtifactResolver,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        let destination = self.root.join(&artifact.sha256);
        match tokio::fs::symlink_metadata(&destination).await {
            // The entry was verified as it was published, and nothing else
            // writes this private directory, so a hit is not read again:
            // every service start would otherwise hash the whole executable.
            // Damage its metadata shows fails the install; it is not repaired.
            Ok(metadata) => {
                if !metadata.is_file() {
                    return Err(demi_artifact::Error::Digest.into());
                }
                if metadata.len() != artifact.size {
                    return Err(demi_artifact::Error::Size {
                        declared: artifact.size,
                        actual: metadata.len(),
                    }
                    .into());
                }
                return Ok(destination);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let expected = Digest {
            size: artifact.size,
            sha256: artifact.sha256.clone(),
        };
        // Out of open files, the attempt waits for one; the five minutes
        // count only an attempt that has its files (`runner.md` § Load).
        let mut backoff = demi_command_service::descriptors::Backoff::default();
        loop {
            let attempt = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(RuntimeError::Cancelled),
                result = tokio::time::timeout(
                    DOWNLOAD_TIMEOUT,
                    self.obtain(artifact, &expected, &destination, resolver, cancel),
                ) => result.unwrap_or(Err(RuntimeError::Deadline("artifact download"))),
            };
            match attempt {
                Err(error) if out_of_files(&error) => tokio::select! {
                    _ = cancel.cancelled() => return Err(error),
                    _ = backoff.wait() => {}
                },
                attempt => return attempt,
            }
        }
    }

    /// One attempt at an executable the cache lacks: the image's copy when
    /// it matches, or else a download to `destination`.
    async fn obtain(
        &self,
        artifact: &PackageArtifact,
        expected: &Digest,
        destination: &Path,
        resolver: &dyn ArtifactResolver,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, RuntimeError> {
        if let Some(executable) = self.preinstalled(expected, cancel).await? {
            return Ok(executable);
        }
        self.download(artifact, expected, destination, resolver, cancel)
            .await?;
        Ok(destination.to_owned())
    }

    /// The image's copy of the executable, when it matches `expected`. The
    /// process checks each copy once, the first time it needs it: one that
    /// matched is used unread from then on, and one that did not is written
    /// to the Host log and passed over. A check that was cancelled or ran
    /// out of open files decides nothing.
    async fn preinstalled(
        &self,
        expected: &Digest,
        cancel: &CancellationToken,
    ) -> Result<Option<PathBuf>, RuntimeError> {
        let Some(image) = &self.image else {
            return Ok(None);
        };
        if let Some(checked) = self.checked().get(&expected.sha256) {
            return Ok(checked.clone());
        }
        let directory = image.join(&expected.sha256);
        let checked = match check(&directory, expected, cancel).await {
            Ok(executable) => Some(executable),
            Err(_) if cancel.is_cancelled() => return Err(RuntimeError::Cancelled),
            Err(error) if out_of_files(&error) => return Err(error),
            // The image holds no copy, as on every paired device.
            Err(RuntimeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => {
                tracing::warn!(
                    "preinstalled executable in {} not used, downloading it: {error}",
                    directory.display()
                );
                None
            }
        };
        self.checked()
            .insert(expected.sha256.clone(), checked.clone());
        Ok(checked)
    }

    fn checked(&self) -> MutexGuard<'_, Checked> {
        self.checked.lock().expect("the checked copies are intact")
    }

    /// Fetches the artifact into a staged file beside `destination` and
    /// publishes it there once verified; any failure removes the staged file.
    async fn download(
        &self,
        artifact: &PackageArtifact,
        expected: &Digest,
        destination: &Path,
        resolver: &dyn ArtifactResolver,
        cancel: &CancellationToken,
    ) -> Result<(), RuntimeError> {
        let mut staged = Staged::new(
            destination,
            Publication {
                mode: Mode::CreateNew,
                permissions: Permissions::Executable,
                durable: true,
            },
        )
        .await?;
        // A URL that expired on the way is asked for once more.
        let mut refreshed = false;
        loop {
            match resolver.resolve(artifact, cancel).await? {
                ArtifactSource::Local(path) => {
                    let mut input = tokio::fs::File::open(path).await?;
                    demi_artifact::copy(&mut input, expected, staged.file(), cancel).await?;
                }
                ArtifactSource::Url { url, expires_at } => {
                    let expired = || expires_at.is_some_and(|expires| expires <= SystemTime::now());
                    if expired() {
                        if refreshed {
                            return Err(RuntimeError::Location(
                                "the backend returned an expired URL".into(),
                            ));
                        }
                        refreshed = true;
                        continue;
                    }
                    let downloaded =
                        demi_artifact::download(&self.http, &url, expected, staged.file(), cancel)
                            .await;
                    match downloaded {
                        Err(demi_artifact::Error::Rejected { status: 403 })
                            if !refreshed && expired() =>
                        {
                            refreshed = true;
                            continue;
                        }
                        downloaded => downloaded?,
                    }
                }
            }
            break;
        }
        staged.publish().await?;
        Ok(())
    }
}

/// The image's copy in `directory`: the directory's one entry, a regular
/// file whose size and SHA-256 are the `expected` ones. A descriptor names
/// no file, so the copy is whatever the directory holds.
async fn check(
    directory: &Path,
    expected: &Digest,
    cancel: &CancellationToken,
) -> Result<PathBuf, RuntimeError> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    let first = entries.next_entry().await?;
    let second = entries.next_entry().await?;
    let (Some(entry), None) = (first, second) else {
        return Err(io::Error::other("the directory does not hold exactly one file").into());
    };
    if !entry.file_type().await?.is_file() {
        return Err(io::Error::other("the directory's entry is not a regular file").into());
    }
    let executable = entry.path();
    let mut input = tokio::fs::File::open(&executable).await?;
    // Checked as a download is, with nowhere to write the bytes to.
    demi_artifact::copy(&mut input, expected, &mut tokio::io::sink(), cancel).await?;
    Ok(executable)
}

/// Whether the attempt failed for want of an open file.
fn out_of_files(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::Io(error) | RuntimeError::Artifact(demi_artifact::Error::Io(error)) => {
            demi_command_service::descriptors::exhausted(error)
        }
        _ => false,
    }
}
