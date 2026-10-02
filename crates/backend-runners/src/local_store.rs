//! The development store (`native-runtime.md` § Backend deployment
//! configuration): a backend on a developer's own machine serves the
//! artifacts of the development releases it loaded itself, each at
//! `/native-artifacts/<sha256>` on its public URL, and answers a runner's
//! location request with that URL. A paired device's runner and the Cloud's
//! then download and verify an artifact as they would from object storage,
//! in the same content coding: the store encodes each executable once, when
//! a runner first asks for it, so a backend's start waits for no encoding,
//! and serves a resource's archive as it is.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use demi_backend_remote_host::ArtifactResolver;
use demi_command_protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::public_url::PublicUrl;

/// Where a development store's executables download from, at the root of
/// the backend's public URL.
pub const ROUTE: &str = "/native-artifacts";

/// The artifacts of the loaded development releases, by SHA-256.
pub struct LocalArtifacts {
    files: HashMap<String, LocalFile>,
}

/// One artifact: its file in its release directory, its size, and for an
/// executable its bytes in the content coding once a runner asked for them.
struct LocalFile {
    path: PathBuf,
    size: u64,
    /// None for a resource's archive, which is served as it is.
    encoded: Option<tokio::sync::OnceCell<Bytes>>,
}

/// How the store serves an artifact.
pub enum LocalArtifact {
    /// An executable's bytes in the content coding
    /// ([`demi_shared_artifacts::CONTENT_CODING`]).
    Encoded(Bytes),
    /// A resource's archive, the file as it is.
    Plain(PathBuf),
}

impl LocalArtifacts {
    /// The store of `executables` and resource `archives`, each a release's
    /// file and the artifact its descriptor declares; one that several
    /// releases carry is kept once.
    pub fn new(
        executables: impl IntoIterator<Item = (PathBuf, PackageArtifact)>,
        archives: impl IntoIterator<Item = (PathBuf, PackageArtifact)>,
    ) -> Self {
        let mut files = HashMap::new();
        for (path, artifact) in executables {
            files.entry(artifact.sha256).or_insert(LocalFile {
                path,
                size: artifact.size,
                encoded: Some(tokio::sync::OnceCell::new()),
            });
        }
        for (path, artifact) in archives {
            files.entry(artifact.sha256).or_insert(LocalFile {
                path,
                size: artifact.size,
                encoded: None,
            });
        }
        Self { files }
    }

    /// The artifact whose SHA-256 is `sha256`, when a loaded release
    /// carries it. Concurrent first requests for an executable share one
    /// encoding. The backend verified the file when it loaded the release:
    /// a file that is gone since is the deployment's fault.
    pub async fn artifact(&self, sha256: &str) -> Option<io::Result<LocalArtifact>> {
        let file = self.files.get(sha256)?;
        let Some(encoded) = &file.encoded else {
            return Some(Ok(LocalArtifact::Plain(file.path.clone())));
        };
        let encoded = encoded
            .get_or_try_init(|| {
                let path = file.path.clone();
                async move {
                    tokio::task::spawn_blocking(move || {
                        let bytes = std::fs::read(&path).map_err(|error| {
                            io::Error::new(error.kind(), format!("{}: {error}", path.display()))
                        })?;
                        demi_shared_artifacts::encode_blocking(
                            &bytes,
                            demi_shared_artifacts::Effort::Development,
                        )
                        .map(Bytes::from)
                        .map_err(io::Error::other)
                    })
                    .await
                    .map_err(io::Error::other)?
                }
            })
            .await;
        Some(encoded.cloned().map(LocalArtifact::Encoded))
    }

    /// Where a runner downloads `artifact`: from `backend`, which serves it.
    fn location(
        &self,
        artifact: &PackageArtifact,
        backend: &PublicUrl,
    ) -> Result<ArtifactLocation, String> {
        let carried = self
            .files
            .get(&artifact.sha256)
            .is_some_and(|file| file.size == artifact.size);
        if !carried {
            return Err("the artifact is not in a loaded development release".into());
        }
        let backend = backend.get().ok_or("the backend does not listen yet")?;
        let origin = backend.url().origin().ascii_serialization();
        Ok(ArtifactLocation::Url(ArtifactUrl {
            url: format!("{origin}{ROUTE}/{}", artifact.sha256),
            expires_at: None,
        }))
    }
}

/// The resolver of one thread's work: the store's executables, at the URL
/// of the backend that serves them.
pub struct ServedArtifacts {
    pub artifacts: Arc<LocalArtifacts>,
    pub backend: PublicUrl,
}

impl ArtifactResolver for ServedArtifacts {
    fn resolve(
        &self,
        artifact: &PackageArtifact,
        _target: &str,
        _cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        let location = self.artifacts.location(artifact, &self.backend);
        Box::pin(std::future::ready(location))
    }
}
