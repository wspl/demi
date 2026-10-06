//! The command packages the conversations' commands bind to
//! (`native-runtime.md` § Publish packages, then source artifacts on demand,
//! § Backend deployment configuration): the descriptors of the published
//! releases, and where a runner downloads each package's executable. The
//! artifact module makes the catalog at startup from the server release's
//! `commands/`; an executable enters the store the first time a runner asks
//! for it, and a runner then downloads it from S3 through a URL signed for
//! five minutes or, with a local store, from this backend, which serves the
//! stored object at `/native-artifacts/<sha256>`. The runner executables
//! are sourced the same way, as their compressed copies. Each shard thread builds its own
//! `CommandCatalog` from the catalog, since a catalog's artifact resolver
//! lives on one thread.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use demi_backend_remote_host::{ArtifactResolver, CommandCatalog};
use demi_command_protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact, PackageDescriptor};
use demi_runner_protocol::manifest::ManifestError;
use demi_backend_database::control::ControlService;
use demi_runner_protocol::release::{RUNNER, compressed_file};
use demi_runner_protocol::wire::RunnerPlatform;
use futures_util::future::LocalBoxFuture;
use object_store::signer::Signer;
use object_store::{GetResult, ObjectStoreExt as _};
use tokio_util::sync::CancellationToken;

use crate::install::read_runner_release;
use crate::public_url::PublicUrl;
use crate::publication::blob;
use crate::sourcing::{ReleaseArtifact, Sourcing};

/// Where a local store's artifacts download from, at the root of the
/// backend's public URL.
pub const ROUTE: &str = "/native-artifacts";

/// How long a runner's signed download URL stays valid.
const SIGNED_FOR: Duration = Duration::from_secs(300);

/// The loaded command packages, as each shard's catalog is built from them.
#[derive(Clone)]
pub struct NativeCatalog {
    packages: Vec<PackageDescriptor>,
    /// None for a backend that loaded no server release.
    artifacts: Option<Arc<Artifacts>>,
}

/// The release's artifacts: the store that holds them once sourced, and
/// the signer of S3 downloads, without which this backend serves them.
pub struct Artifacts {
    sourcing: Sourcing,
    signer: Option<Arc<dyn Signer>>,
    /// Each package executable, by SHA-256.
    executables: HashMap<String, ReleaseArtifact>,
}

impl Artifacts {
    pub fn new(
        sourcing: Sourcing,
        signer: Option<Arc<dyn Signer>>,
        executables: HashMap<String, ReleaseArtifact>,
    ) -> Self {
        Self {
            sourcing,
            signer,
            executables,
        }
    }

    /// Where a runner downloads `artifact`, a package's executable, once the
    /// store holds it: a signed S3 URL, or this backend at `backend`.
    async fn location(
        &self,
        artifact: &PackageArtifact,
        backend: &PublicUrl,
        cancel: &CancellationToken,
    ) -> Result<ArtifactLocation, String> {
        let wanted = self
            .executables
            .get(&artifact.sha256)
            .filter(|wanted| wanted.artifact.size == artifact.size)
            .ok_or("the artifact is not in the published package catalog")?;
        self.sourcing.ensure(wanted, cancel).await?;
        let key = blob(&artifact.sha256).map_err(|error| error.to_string())?;
        let Some(signer) = &self.signer else {
            let backend = backend.get().ok_or("the backend does not listen yet")?;
            let origin = backend.url().origin().ascii_serialization();
            return Ok(ArtifactLocation::Url(ArtifactUrl {
                url: format!("{origin}{ROUTE}/{}", artifact.sha256),
                expires_at: None,
            }));
        };
        let url = signer
            .signed_url(Method::GET, &key, SIGNED_FOR)
            .await
            .map_err(|error| error.to_string())?;
        if url.scheme() != "https" {
            return Err("a native artifact downloads over HTTPS only".into());
        }
        let expires_at = jiff::Timestamp::now() + SIGNED_FOR;
        Ok(ArtifactLocation::Url(ArtifactUrl {
            url: url.into(),
            expires_at: Some(expires_at.as_millisecond()),
        }))
    }
}

impl NativeCatalog {
    /// The catalog of `packages`, whose executables `artifacts` sources. It
    /// is refused when a descriptor is invalid or two packages share an id.
    pub fn new(packages: Vec<PackageDescriptor>, artifacts: Artifacts) -> Result<Self, ManifestError> {
        CommandCatalog::new(packages.clone(), Rc::new(Unpublished))?;
        Ok(Self {
            packages,
            artifacts: Some(Arc::new(artifacts)),
        })
    }

    /// No package and no release: a command set that declares a native
    /// command selects no manifest, so a node whose commands include one
    /// gets no shell, and no runner executable is served. A test's backend
    /// runs on it unless it loads a server release; the product's start
    /// publishes its server release instead (`publish_native`).
    pub fn unpublished() -> Self {
        Self {
            packages: Vec::new(),
            artifacts: None,
        }
    }

    /// The calling thread's catalog; a local store's downloads are on
    /// `backend`.
    pub fn catalog(&self, backend: &PublicUrl) -> CommandCatalog {
        CommandCatalog::new(self.packages.clone(), self.resolver(backend))
            .expect("the packages were checked when the catalog was made")
    }

    /// The calling thread's resolver of the packages' executables, for the
    /// work that binds a package without a manifest: a user stream or a
    /// one-shot user call. A local store's downloads are on `backend`.
    pub fn resolver(&self, backend: &PublicUrl) -> Rc<dyn ArtifactResolver> {
        match &self.artifacts {
            None => Rc::new(Unpublished),
            Some(artifacts) => Rc::new(Resolver {
                artifacts: artifacts.clone(),
                backend: backend.clone(),
            }),
        }
    }

    /// The stored object of the package executable whose SHA-256 is
    /// `sha256`, when this backend serves it: with a local store, once a
    /// runner's need put it there.
    pub async fn local_artifact(&self, sha256: &str) -> Option<object_store::Result<GetResult>> {
        let artifacts = self.artifacts.as_ref()?;
        if artifacts.signer.is_some() || !artifacts.executables.contains_key(sha256) {
            return None;
        }
        let key = match blob(sha256) {
            Ok(key) => key,
            Err(error) => {
                return Some(Err(object_store::Error::Generic {
                    store: "the object store",
                    source: error.into(),
                }));
            }
        };
        match artifacts.sourcing.store().get(&key).await {
            Err(object_store::Error::NotFound { .. }) => None,
            got => Some(got),
        }
    }

    /// Makes sure the store holds the runner executable `artifact` of
    /// `target`, a runner release's, as its compressed copy, taking it from
    /// the release's files when it does not (`native-runtime.md` § Runner
    /// releases).
    pub async fn source_runner(
        &self,
        target: &str,
        artifact: &PackageArtifact,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let artifacts = self
            .artifacts
            .as_ref()
            .ok_or("this backend loaded no server release")?;
        let wanted = ReleaseArtifact {
            file: compressed_file(RUNNER, target),
            artifact: artifact.clone(),
            encoded: true,
        };
        artifacts.sourcing.ensure(&wanted, cancel).await
    }

    /// The stored runner executable `artifact` of `target`, its compressed
    /// copy in the content coding, sourced first when the store lacks it,
    /// for a runner's update or an installer's download.
    pub async fn runner_executable(
        &self,
        target: &str,
        artifact: &PackageArtifact,
        cancel: &CancellationToken,
    ) -> Result<GetResult, String> {
        self.source_runner(target, artifact, cancel).await?;
        let artifacts = self
            .artifacts
            .as_ref()
            .ok_or("this backend loaded no server release")?;
        let key = blob(&artifact.sha256).map_err(|error| error.to_string())?;
        artifacts
            .sourcing
            .store()
            .get(&key)
            .await
            .map_err(|error| error.to_string())
    }

    /// Sources the runner executables of the current release in `runners`,
    /// a server release's `runners/`, for the targets of the systems of the
    /// paired devices `control` lists, one after another, so that an update
    /// never waits on the release's origin (`native-runtime.md` § Runner
    /// releases). The device records name a system, not a target, so each
    /// of the system's targets is sourced. A failure is logged: a runner's
    /// update sources its executable again.
    pub async fn source_paired_runners(
        &self,
        runners: &Path,
        control: &ControlService,
        cancel: &CancellationToken,
    ) {
        let release = match read_runner_release(&runners.join("manifest.json")).await {
            Ok(Some(release)) => release,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!("the runner release to source cannot be read: {error}");
                return;
            }
        };
        let platforms = match control.paired_platforms().await {
            Ok(platforms) => platforms,
            Err(error) => {
                tracing::warn!(
                    error = &error as &dyn std::error::Error,
                    "the paired devices' systems cannot be listed"
                );
                return;
            }
        };
        for (target, artifact) in &release.targets {
            if !platforms.iter().any(|platform| runs(*platform, target)) {
                continue;
            }
            match self.source_runner(target, artifact, cancel).await {
                Ok(()) => tracing::info!(target = %target, "the runner executable is in the store"),
                Err(error) => {
                    tracing::warn!(target = %target, "the runner executable was not sourced: {error}")
                }
            }
        }
    }

    /// The loaded package `id`.
    pub fn package(&self, id: &str) -> Option<&PackageDescriptor> {
        self.packages.iter().find(|descriptor| descriptor.id == id)
    }

    /// Whether `package` serves every one of `operations`.
    pub fn serves(&self, package: &str, operations: &[&str]) -> bool {
        self.package(package).is_some_and(|descriptor| {
            operations.iter().all(|operation| {
                descriptor
                    .operations
                    .iter()
                    .any(|served| served == operation)
            })
        })
    }
}

/// Whether a runner for `target` runs on a device of `platform`.
fn runs(platform: RunnerPlatform, target: &str) -> bool {
    match platform {
        RunnerPlatform::Darwin => target.contains("apple-darwin"),
        RunnerPlatform::Win32 => target.contains("windows"),
        RunnerPlatform::Linux => target.contains("linux"),
    }
}

/// One thread's resolver: the release's artifacts, sourced on demand, at
/// the URL of the backend that serves a local store's.
struct Resolver {
    artifacts: Arc<Artifacts>,
    backend: PublicUrl,
}

impl ArtifactResolver for Resolver {
    fn resolve(
        &self,
        artifact: &PackageArtifact,
        _target: &str,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        let artifacts = self.artifacts.clone();
        let backend = self.backend.clone();
        let artifact = artifact.clone();
        Box::pin(async move {
            tokio::select! {
                () = cancel.cancelled() => Err("the work that asked no longer runs".into()),
                location = artifacts.location(&artifact, &backend, &cancel) => location,
            }
        })
    }
}

/// The resolver of a catalog with no package, which nothing asks.
struct Unpublished;

impl ArtifactResolver for Unpublished {
    fn resolve(
        &self,
        _artifact: &PackageArtifact,
        target: &str,
        _cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        let message = format!("no command package is published for {target}");
        Box::pin(async move { Err(message) })
    }
}
