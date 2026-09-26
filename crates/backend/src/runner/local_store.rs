//! The development store (`native-runtime.md` § Backend deployment
//! configuration): a backend on a developer's own machine serves the
//! executables of the development releases it loaded itself, each at
//! `/native-artifacts/<sha256>` on its public URL, and answers a runner's
//! location request with that URL. A paired device's runner and the Cloud's
//! then download and verify an executable as they would from object storage.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_command_service::protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact};
use demi_host_remote::ArtifactResolver;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use crate::backend::PublicUrl;

/// Where a development store's executables download from, at the root of
/// the backend's public URL.
pub(crate) const ROUTE: &str = "/native-artifacts";

/// The executables of the loaded development releases, by SHA-256.
pub(crate) struct LocalArtifacts {
    files: HashMap<String, LocalFile>,
}

/// One executable: its file in its release directory, and its size.
struct LocalFile {
    path: PathBuf,
    size: u64,
}

impl LocalArtifacts {
    /// The store of `executables`, each a release's file and the artifact
    /// its descriptor declares; one that several releases carry is kept
    /// once.
    pub(crate) fn new(executables: impl IntoIterator<Item = (PathBuf, PackageArtifact)>) -> Self {
        let mut files = HashMap::new();
        for (path, artifact) in executables {
            files.entry(artifact.sha256).or_insert(LocalFile {
                path,
                size: artifact.size,
            });
        }
        Self { files }
    }

    /// The file of the executable whose SHA-256 is `sha256`, when a loaded
    /// release carries it.
    pub(crate) fn file(&self, sha256: &str) -> Option<&Path> {
        self.files.get(sha256).map(|file| file.path.as_path())
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
pub(crate) struct ServedArtifacts {
    pub(crate) artifacts: Arc<LocalArtifacts>,
    pub(crate) backend: PublicUrl,
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
