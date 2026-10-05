//! A local store's downloads (`native-runtime.md` § Backend deployment
//! configuration): the data directory is reachable only through the
//! backend, so the backend serves the command artifacts it published there
//! itself, each at `/native-artifacts/<sha256>` on its public URL, and
//! answers a runner's location request with that URL. A paired device's
//! runner and the Cloud's then download and verify an artifact as they would
//! from S3, in the content coding it was stored with.

use std::collections::HashMap;
use std::sync::Arc;

use demi_backend_remote_host::ArtifactResolver;
use demi_command_protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact};
use futures_util::future::LocalBoxFuture;
use object_store::{GetResult, ObjectStore, ObjectStoreExt as _};
use tokio_util::sync::CancellationToken;

use crate::public_url::PublicUrl;

/// Where a local store's artifacts download from, at the root of the
/// backend's public URL.
pub const ROUTE: &str = "/native-artifacts";

/// The artifacts published to a local store.
pub struct LocalArtifacts {
    store: Arc<dyn ObjectStore>,
    /// Each published artifact's size, by SHA-256.
    published: HashMap<String, u64>,
}

impl LocalArtifacts {
    /// The artifacts `published` names, which `store` holds.
    pub fn new(store: Arc<dyn ObjectStore>, published: HashMap<String, u64>) -> Self {
        Self { store, published }
    }

    /// The object of the published artifact whose SHA-256 is `sha256`, with
    /// the attributes it was stored with; none for any other digest.
    pub async fn artifact(&self, sha256: &str) -> Option<object_store::Result<GetResult>> {
        if !self.published.contains_key(sha256) {
            return None;
        }
        let key = match crate::publication::blob(sha256) {
            Ok(key) => key,
            Err(error) => {
                return Some(Err(object_store::Error::Generic {
                    store: "the local object store",
                    source: error.into(),
                }));
            }
        };
        Some(self.store.get(&key).await)
    }

    /// Where a runner downloads `artifact`: from `backend`, which serves it.
    fn location(
        &self,
        artifact: &PackageArtifact,
        backend: &PublicUrl,
    ) -> Result<ArtifactLocation, String> {
        if self.published.get(&artifact.sha256) != Some(&artifact.size) {
            return Err("the artifact is not in the published package catalog".into());
        }
        let backend = backend.get().ok_or("the backend does not listen yet")?;
        let origin = backend.url().origin().ascii_serialization();
        Ok(ArtifactLocation::Url(ArtifactUrl {
            url: format!("{origin}{ROUTE}/{}", artifact.sha256),
            expires_at: None,
        }))
    }
}

/// The resolver of one thread's work: the store's artifacts, at the URL of
/// the backend that serves them.
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
