use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use demi_command_service::protocol::PackageArtifact;
use futures_util::{StreamExt, future::BoxFuture};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, watch},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("native artifact: {0}")]
    Artifact(String),
    #[error("native runtime was cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Service(#[from] demi_command_service::ServiceError),
    #[error("native service does not match its package descriptor")]
    CatalogMismatch,
    #[error("native service exceeded its {0} deadline")]
    Deadline(&'static str),
}

pub enum ArtifactSource {
    Local(PathBuf),
    Https {
        url: String,
        expires_at: Option<std::time::SystemTime>,
    },
}

impl ArtifactSource {
    /// Convert a catalog-authorized artifact location into a cache download source.
    pub fn from_location(
        location: demi_command_service::protocol::ArtifactLocation,
    ) -> Result<Self, RuntimeError> {
        match location {
            demi_command_service::protocol::ArtifactLocation::Url(location) => {
                let url = reqwest::Url::parse(&location.url)
                    .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
                if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
                {
                    return Err(RuntimeError::Artifact(
                        "artifact downloads require an HTTPS URL without credentials".into(),
                    ));
                }
                let expires_at = location
                    .expires_at
                    .map(|millis| {
                        u64::try_from(millis)
                            .ok()
                            .and_then(|millis| {
                                std::time::UNIX_EPOCH
                                    .checked_add(std::time::Duration::from_millis(millis))
                            })
                            .ok_or_else(|| {
                                RuntimeError::Artifact("invalid artifact URL expiry".into())
                            })
                    })
                    .transpose()?;
                Ok(ArtifactSource::Https {
                    url: url.into(),
                    expires_at,
                })
            }
            demi_command_service::protocol::ArtifactLocation::Path(location) => {
                let path = std::path::PathBuf::from(location.path);
                if !path.is_absolute() {
                    return Err(RuntimeError::Artifact(
                        "local artifact path must be absolute".into(),
                    ));
                }
                Ok(ArtifactSource::Local(path))
            }
        }
    }
}

/// Resolves only an artifact authorized by the calling registration's catalog.
/// URLs are resolved again for each download attempt and never become cache keys.
pub trait ArtifactResolver: Send + Sync + 'static {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>>;
}

type DownloadResult = Result<PathBuf, Arc<RuntimeError>>;
struct Flight {
    size: u64,
    receiver: watch::Receiver<Option<DownloadResult>>,
}

struct CacheState {
    flights: HashMap<String, Flight>,
    closed: bool,
}

pub struct ArtifactCache {
    root: PathBuf,
    http: reqwest::Client,
    state: Mutex<CacheState>,
    cancel: CancellationToken,
    tasks: TaskTracker,
}

impl ArtifactCache {
    pub async fn new(root: PathBuf) -> Result<Arc<Self>, RuntimeError> {
        tokio::fs::create_dir_all(&root).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).await?;
        }
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
        Ok(Arc::new(Self {
            root,
            http,
            state: Mutex::new(CacheState {
                flights: HashMap::new(),
                closed: false,
            }),
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }))
    }

    pub async fn acquire(
        self: &Arc<Self>,
        artifact: PackageArtifact,
        resolver: Arc<dyn ArtifactResolver>,
        caller_cancel: &CancellationToken,
    ) -> DownloadResult {
        if artifact.size == 0
            || artifact.sha256.len() != 64
            || !artifact
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Arc::new(RuntimeError::Artifact(
                "invalid artifact descriptor".into(),
            )));
        }
        let mut receiver = {
            let mut state = self.state.lock().await;
            if state.closed || self.cancel.is_cancelled() {
                return Err(Arc::new(RuntimeError::Cancelled));
            }
            if let Some(flight) = state.flights.get(&artifact.sha256) {
                if flight.size != artifact.size {
                    return Err(Arc::new(RuntimeError::Artifact(
                        "conflicting artifact sizes".into(),
                    )));
                }
                flight.receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                state.flights.insert(
                    artifact.sha256.clone(),
                    Flight {
                        size: artifact.size,
                        receiver: receiver.clone(),
                    },
                );
                let cache = self.clone();
                self.tasks.spawn(async move {
                    // Out of open files, the download waits for one; the five
                    // minutes count only an attempt that has its files
                    // (`runner.md` § Load).
                    let mut backoff = demi_command_service::descriptors::Backoff::default();
                    let result = loop {
                        let attempt = tokio::select! {
                            biased;
                            _ = cache.cancel.cancelled() => Err(RuntimeError::Cancelled),
                            result = tokio::time::timeout(std::time::Duration::from_secs(300), cache.download(&artifact, resolver.as_ref())) => {
                                result.unwrap_or_else(|_| Err(RuntimeError::Deadline("artifact download")))
                            },
                        };
                        match attempt {
                            Err(RuntimeError::Io(error)) if demi_command_service::descriptors::exhausted(&error) => {
                                backoff.wait().await
                            }
                            other => break other,
                        }
                    }.map_err(Arc::new);
                    sender.send_replace(Some(result));
                    cache.state.lock().await.flights.remove(&artifact.sha256);
                });
                receiver
            }
        };
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                return result;
            }
            tokio::select! {
                biased;
                _ = caller_cancel.cancelled() => return Err(Arc::new(RuntimeError::Cancelled)),
                changed = receiver.changed() => {
                    if changed.is_err() { return Err(Arc::new(RuntimeError::Cancelled)); }
                }
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
        self.tasks.close();
    }

    pub async fn shutdown(&self) {
        self.state.lock().await.closed = true;
        self.cancel();
        self.tasks.wait().await;
    }

    async fn download(
        &self,
        artifact: &PackageArtifact,
        resolver: &dyn ArtifactResolver,
    ) -> Result<PathBuf, RuntimeError> {
        let destination = self.root.join(&artifact.sha256);
        match verify_file(&destination, artifact).await {
            Ok(()) => return Ok(destination),
            Err(RuntimeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        // TempPath removes incomplete downloads on every error and cancellation path.
        let temporary = tempfile::NamedTempFile::new_in(&self.root)?.into_temp_path();
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&temporary)
            .await?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut refreshed = false;
        loop {
            match resolver.resolve(artifact, &self.cancel).await? {
                ArtifactSource::Local(path) => {
                    let mut input = tokio::fs::File::open(path).await?;
                    let mut buffer = vec![0; 64 * 1024];
                    loop {
                        let count = input.read(&mut buffer).await?;
                        if count == 0 {
                            break;
                        }
                        write_chunk(
                            &mut output,
                            &mut hash,
                            &mut size,
                            artifact.size,
                            &buffer[..count],
                        )
                        .await?;
                    }
                }
                ArtifactSource::Https { url, expires_at } => {
                    if expires_at.is_some_and(|expires| expires <= std::time::SystemTime::now()) {
                        if refreshed {
                            return Err(RuntimeError::Artifact(
                                "artifact resolver returned an expired URL".into(),
                            ));
                        }
                        refreshed = true;
                        continue;
                    }
                    let response = self
                        .http
                        .get(url)
                        .send()
                        .await
                        .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
                    if !refreshed
                        && response.status() == reqwest::StatusCode::FORBIDDEN
                        && expires_at.is_some_and(|expires| expires <= std::time::SystemTime::now())
                    {
                        refreshed = true;
                        continue;
                    }
                    let response = response
                        .error_for_status()
                        .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
                    if response
                        .content_length()
                        .is_some_and(|size| size != artifact.size)
                    {
                        return Err(RuntimeError::Artifact(
                            "artifact Content-Length mismatch".into(),
                        ));
                    }
                    let mut stream = response.bytes_stream();
                    while let Some(chunk) = stream.next().await {
                        let chunk =
                            chunk.map_err(|error| RuntimeError::Artifact(error.to_string()))?;
                        write_chunk(&mut output, &mut hash, &mut size, artifact.size, &chunk)
                            .await?;
                    }
                }
            }
            break;
        }
        check_digest(hash, size, artifact)?;
        output.sync_all().await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output
                .set_permissions(std::fs::Permissions::from_mode(0o700))
                .await?;
        }
        drop(output);
        match temporary.persist_noclobber(&destination) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                verify_file(&destination, artifact).await?;
            }
            Err(error) => return Err(error.error.into()),
        }
        Ok(destination)
    }
}

async fn write_chunk(
    output: &mut tokio::fs::File,
    hash: &mut Sha256,
    size: &mut u64,
    limit: u64,
    chunk: &[u8],
) -> Result<(), RuntimeError> {
    *size = size
        .checked_add(chunk.len() as u64)
        .filter(|size| *size <= limit)
        .ok_or_else(|| RuntimeError::Artifact("artifact exceeds declared size".into()))?;
    hash.update(chunk);
    output.write_all(chunk).await?;
    Ok(())
}

async fn verify_file(path: &Path, artifact: &PackageArtifact) -> Result<(), RuntimeError> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if !metadata.is_file() || metadata.len() != artifact.size {
        return Err(RuntimeError::Artifact(
            "cached artifact size or file type mismatch".into(),
        ));
    }
    let actual = demi_artifact::digest(path, artifact.size, &CancellationToken::new())
        .await
        .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
    if actual.size != artifact.size || actual.sha256 != artifact.sha256 {
        return Err(RuntimeError::Artifact(
            "artifact size or SHA-256 mismatch".into(),
        ));
    }
    Ok(())
}

fn check_digest(hash: Sha256, size: u64, artifact: &PackageArtifact) -> Result<(), RuntimeError> {
    if size != artifact.size || format!("{:x}", hash.finalize()) != artifact.sha256 {
        return Err(RuntimeError::Artifact(
            "artifact size or SHA-256 mismatch".into(),
        ));
    }
    Ok(())
}
