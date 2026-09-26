//! The verified executable cache (`native-runtime.md` § Install the selected
//! executable): one file per artifact digest, published only once its size
//! and SHA-256 match what the descriptor declares, and reused unread from
//! then on. The service registry starts one install per digest at a time, so
//! concurrent callers share it and the cache keeps no state of its own.

use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use demi_artifact::{Digest, Mode, Permissions, Publication, Staged};
use demi_command_service::protocol::PackageArtifact;
use tokio_util::sync::CancellationToken;

use super::{ArtifactResolver, ArtifactSource, RuntimeError};

/// How long one download attempt may take once it has its files.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

pub struct ArtifactCache {
    root: PathBuf,
    http: reqwest::Client,
}

impl ArtifactCache {
    pub async fn new(root: PathBuf) -> Result<Self, RuntimeError> {
        tokio::fs::create_dir_all(&root).await?;
        crate::fs::chmod(&root, 0o700).await?;
        Ok(Self {
            root,
            http: demi_artifact::client()?,
        })
    }

    /// The cached executable for `artifact`, downloaded on a miss. A cached
    /// entry that is not a regular file of the declared size fails the
    /// install.
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
        // Out of open files, the download waits for one; the five minutes
        // count only an attempt that has its files (`runner.md` § Load).
        let mut backoff = demi_command_service::descriptors::Backoff::default();
        loop {
            let attempt = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(RuntimeError::Cancelled),
                result = tokio::time::timeout(
                    DOWNLOAD_TIMEOUT,
                    self.download(artifact, &expected, &destination, resolver, cancel),
                ) => result.unwrap_or(Err(RuntimeError::Deadline("artifact download"))),
            };
            match attempt {
                Err(error) if out_of_files(&error) => tokio::select! {
                    _ = cancel.cancelled() => return Err(error),
                    _ = backoff.wait() => {}
                },
                attempt => return attempt.map(|()| destination),
            }
        }
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
                ArtifactSource::Https { url, expires_at } => {
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

/// Whether the attempt failed for want of an open file.
fn out_of_files(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::Io(error) | RuntimeError::Artifact(demi_artifact::Error::Io(error)) => {
            demi_command_service::descriptors::exhausted(error)
        }
        _ => false,
    }
}
