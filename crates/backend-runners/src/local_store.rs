//! The development store (`native-runtime.md` § Backend deployment
//! configuration): a backend on a developer's own machine serves the
//! executables of the development releases it loaded itself, each at
//! `/native-artifacts/<sha256>` on its public URL, and answers a runner's
//! location request with that URL. A paired device's runner and the Cloud's
//! then download and verify an executable as they would from object storage,
//! in the same content coding: the store encodes each executable once, when
//! a runner first asks for it, so a backend's start waits for no encoding.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use demi_command_service::protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact};
use demi_host_remote::ArtifactResolver;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::public_url::PublicUrl;

/// Where a development store's executables download from, at the root of
/// the backend's public URL.
pub const ROUTE: &str = "/native-artifacts";

/// The executables of the loaded development releases, by SHA-256.
pub struct LocalArtifacts {
    files: HashMap<String, LocalFile>,
}

/// One executable: its file in its release directory, its size, and its
/// bytes in the content coding once a runner asked for them.
struct LocalFile {
    path: PathBuf,
    size: u64,
    encoded: tokio::sync::OnceCell<Bytes>,
}

impl LocalArtifacts {
    /// The store of `executables`, each a release's file and the artifact
    /// its descriptor declares; one that several releases carry is kept
    /// once.
    pub fn new(executables: impl IntoIterator<Item = (PathBuf, PackageArtifact)>) -> Self {
        let mut files = HashMap::new();
        for (path, artifact) in executables {
            files.entry(artifact.sha256).or_insert(LocalFile {
                path,
                size: artifact.size,
                encoded: tokio::sync::OnceCell::new(),
            });
        }
        Self { files }
    }

    /// The executable whose SHA-256 is `sha256` in the content coding
    /// ([`demi_artifact::CONTENT_CODING`]), when a loaded release carries it.
    /// Concurrent first requests share one encoding. The backend verified the
    /// file when it loaded the release: a file that is gone since is the
    /// deployment's fault.
    pub async fn encoded(&self, sha256: &str) -> Option<io::Result<Bytes>> {
        let file = self.files.get(sha256)?;
        let encoded = file
            .encoded
            .get_or_try_init(|| {
                let path = file.path.clone();
                async move {
                    tokio::task::spawn_blocking(move || {
                        let bytes = std::fs::read(&path)
                            .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
                        demi_artifact::encode_blocking(&bytes, demi_artifact::Effort::Development)
                            .map(Bytes::from)
                            .map_err(io::Error::other)
                    })
                    .await
                    .map_err(io::Error::other)?
                }
            })
            .await;
        Some(encoded.cloned())
    }

    /// Where a runner downloads `artifact`: from `backend`, which serves it.
    fn location(&self, artifact: &PackageArtifact, backend: &PublicUrl) -> Result<ArtifactLocation, String> {
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
