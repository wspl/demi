//! The command packages the conversations' commands bind to: the descriptors
//! of the published
//! releases, and where a runner downloads each package's executable. The
//! artifact module makes the catalog at startup from the server release's
//! `commands/`; an executable enters the store when the backend stores ahead
//! for the targets of its Hosts, or else the first time a runner asks for
//! it, and a runner then downloads it from S3 through a URL signed for
//! five minutes or, with a local store, from this backend, which serves the
//! stored object at `/native-artifacts/<sha256>`. The runner executables
//! are sourced the same way, as their compressed copies. Each shard thread builds its own
//! `CommandCatalog` from the catalog, since a catalog's artifact resolver
//! lives on one thread. A job that `demi host shell` starts on another Host
//! receives the calling session's catalog.
//!
//! A local store's downloads need no credentials and expire never, like the
//! runner installers': they reveal nothing a user owns and cannot change what
//! runs, since the objects are the released programs, which the release
//! publishes anyway, and each URL names a digest that the runner checks the
//! download against, a digest that reached it over its authenticated
//! connection. A runner's `artifact_location` request has no deadline of its
//! own while its connection lives, since a fetch on a small server's slow disk
//! or link is not a failure; the other questions a runner asks, such as a
//! conversation's numbers, are answered from what the backend holds.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::Method;
use demi_backend_remote_host::{ArtifactResolver, CommandCatalog};
use demi_command_protocol::{
    ArtifactLocation, ArtifactUrl, PackageArtifact, PackageDescriptor, TARGETS,
};
use demi_runner_protocol::manifest::ManifestError;
use demi_backend_database::control::ControlService;
use demi_runner_protocol::release::{RUNNER, RunnerRelease, SERVER_RELEASE, ServerRelease, compressed_file};
use demi_runner_protocol::wire::RunnerPlatform;
use futures_util::future::LocalBoxFuture;
use object_store::signer::Signer;
use object_store::{GetOptions, GetRange, GetResult, ObjectStore, ObjectStoreExt as _};
use tokio_util::sync::CancellationToken;

use crate::install::read_runner_release;
use crate::prefetch::NextRelease;
use crate::public_url::PublicUrl;
use crate::publication::blob;
use crate::sourcing::{ReleaseArtifact, Sourcing};

/// A stored artifact from the byte a download asked for, as one that broke
/// off resumes: the stored bytes, or none when the download asked for a byte
/// past the object's `size`.
pub enum StoredBytes {
    Stored(GetResult),
    Beyond { size: u64 },
}

/// The object at `key` in `store` from its byte `from` on.
async fn stored_from(
    store: &dyn ObjectStore,
    key: &object_store::path::Path,
    from: u64,
) -> object_store::Result<StoredBytes> {
    if from > 0 {
        let size = store.head(key).await?.size;
        if from >= size {
            return Ok(StoredBytes::Beyond { size });
        }
    }
    let options = GetOptions {
        range: (from > 0).then_some(GetRange::Offset(from)),
        ..GetOptions::default()
    };
    store.get_opts(key, options).await.map(StoredBytes::Stored)
}

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
    /// The targets whose executables are stored ahead.
    ahead: Mutex<Ahead>,
    /// Wakes the worker that stores ahead when a target joins the queue.
    queued: tokio::sync::Notify,
}

/// The targets the backend stores the release's package executables
/// ahead for (`crates/backend-runners/src/publication.rs`).
#[derive(Default)]
struct Ahead {
    /// Each target foreseen: queued, being stored or stored. A target whose
    /// storing failed leaves it, so the next Host of the target that pairs
    /// or wakes stores it again.
    foreseen: HashSet<String>,
    /// The foreseen targets the worker has not taken yet, in order.
    queue: VecDeque<String>,
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
            ahead: Mutex::default(),
            queued: tokio::sync::Notify::new(),
        }
    }

    /// The next foreseen target to store for, once one is queued.
    async fn next_target(&self) -> String {
        loop {
            if let Some(target) = self.ahead.lock().expect("the targets are intact").queue.pop_front() {
                return target;
            }
            // A target queued since the check left a permit, which this
            // takes at once.
            self.queued.notified().await;
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
    /// `sha256`, from its byte `from` on, when this backend serves it: with
    /// a local store, once a runner's need put it there.
    pub async fn local_artifact(
        &self,
        sha256: &str,
        from: u64,
    ) -> Option<Result<StoredBytes, String>> {
        let artifacts = self.artifacts.as_ref()?;
        if artifacts.signer.is_some() || !artifacts.executables.contains_key(sha256) {
            return None;
        }
        let key = match blob(sha256) {
            Ok(key) => key,
            Err(error) => return Some(Err(error.to_string())),
        };
        match stored_from(artifacts.sourcing.store().as_ref(), &key, from).await {
            Err(object_store::Error::NotFound { .. }) => None,
            got => Some(got.map_err(|error| error.to_string())),
        }
    }

    /// Makes sure the store holds the runner executable `artifact` of
    /// `target`, a runner release's, as its compressed copy, taking it from
    /// the release's files when it does not (`crates/backend-http/src/install.rs`).
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
    /// copy in the content coding, from its byte `from` on, sourced first
    /// when the store lacks it: from this backend's release, or from the
    /// files of the `next` release an upgrade announced (`crates/backend-http/src/install.rs`). For a runner's update or download ahead, which
    /// resumes from the bytes it has, or an installer's download.
    pub async fn runner_executable(
        &self,
        target: &str,
        artifact: &PackageArtifact,
        next: Option<&NextRelease>,
        from: u64,
        cancel: &CancellationToken,
    ) -> Result<StoredBytes, String> {
        let artifacts = self
            .artifacts
            .as_ref()
            .ok_or("this backend loaded no server release")?;
        let sourcing = next.map_or(&artifacts.sourcing, |next| &next.sourcing);
        let wanted = ReleaseArtifact {
            file: compressed_file(RUNNER, target),
            artifact: artifact.clone(),
            encoded: true,
        };
        sourcing.ensure(&wanted, cancel).await?;
        let key = blob(&artifact.sha256).map_err(|error| error.to_string())?;
        stored_from(sourcing.store().as_ref(), &key, from)
            .await
            .map_err(|error| error.to_string())
    }

    /// The runner release `release` of the server release whose root is
    /// `root`, which an upgrade announced as next
    /// (`crates/server/src/main.rs`): its record from the root's
    /// `runners/`, and its executables sourced into this backend's
    /// store from the files the root's `release.json` names.
    pub async fn next_release(&self, release: &str, root: &Path) -> Result<NextRelease, String> {
        let artifacts = self
            .artifacts
            .as_ref()
            .ok_or("this backend loaded no server release")?;
        let record = root.join("runners").join(release).join("manifest.json");
        // The next release may speak other protocols: only its integrity counts.
        let record = read_runner_release(&record, RunnerRelease::decode_intact)
            .await?
            .filter(|record| record.release == release)
            .ok_or_else(|| format!("{} holds no runner release {release}", root.display()))?;
        let server = root.join(SERVER_RELEASE);
        let server = tokio::fs::read(&server)
            .await
            .map_err(|error| format!("{}: {error}", server.display()))
            .and_then(|bytes| {
                ServerRelease::decode(&bytes).map_err(|error| format!("{}: {error}", server.display()))
            })?;
        let sourcing = Sourcing::new(artifacts.sourcing.store().clone(), server.location())
            .map_err(|error| error.to_string())?;
        Ok(NextRelease::new(record, sourcing))
    }

    /// Stores ahead the package executables of each of `targets` that was
    /// not foreseen before: the targets of the Hosts this backend knows,
    /// as it starts, and the target of a Host that pairs or wakes
    /// (`crates/backend-runners/src/publication.rs`). `store_ahead` stores them, in the background.
    pub fn foresee<'a>(&self, targets: impl IntoIterator<Item = &'a str>) {
        let Some(artifacts) = &self.artifacts else {
            return;
        };
        let mut ahead = artifacts.ahead.lock().expect("the targets are intact");
        let mut queued = false;
        for target in targets {
            if ahead.foreseen.insert(target.to_owned()) {
                ahead.queue.push_back(target.to_owned());
                queued = true;
            }
        }
        drop(ahead);
        if queued {
            artifacts.queued.notify_one();
        }
    }

    /// Stores, one artifact at a time, every package executable of the
    /// release for each target `foresee` queues, until `cancel`. A need of
    /// an artifact being stored shares its fetch. A failure is logged and
    /// stops nothing: the next need of the artifact fetches it again, as
    /// the next foreseeing of its target does.
    pub async fn store_ahead(&self, cancel: &CancellationToken) {
        let Some(artifacts) = &self.artifacts else {
            return;
        };
        loop {
            let target = tokio::select! {
                () = cancel.cancelled() => return,
                target = artifacts.next_target() => target,
            };
            let mut stored = true;
            for descriptor in &self.packages {
                let Some(wanted) = descriptor
                    .targets
                    .get(&target)
                    .and_then(|artifact| artifacts.executables.get(&artifact.sha256))
                else {
                    continue;
                };
                match artifacts.sourcing.ensure(wanted, cancel).await {
                    Ok(()) => tracing::info!(
                        package = %descriptor.id,
                        target = %target,
                        "the package executable is in the store"
                    ),
                    Err(_) if cancel.is_cancelled() => return,
                    Err(error) => {
                        stored = false;
                        tracing::warn!(
                            package = %descriptor.id,
                            target = %target,
                            "the package executable was not stored ahead: {error}"
                        );
                    }
                }
            }
            if !stored {
                artifacts
                    .ahead
                    .lock()
                    .expect("the targets are intact")
                    .foreseen
                    .remove(&target);
            }
        }
    }

    /// Foresees the targets of the systems of the paired devices `control`
    /// lists: the target its runner runs on when it reported its
    /// architecture, and each of the system's targets before it did. A
    /// failure to list them is logged: each device's next connection
    /// foresees its target.
    pub async fn foresee_paired(&self, control: &ControlService) {
        match control.paired_systems().await {
            Ok(systems) => {
                for system in systems {
                    let arch = system.os.as_ref().map(|os| os.arch.as_str());
                    self.foresee(system_targets(system.platform, arch));
                }
            }
            Err(error) => tracing::warn!(
                error = &error as &dyn std::error::Error,
                "the paired devices' systems cannot be listed"
            ),
        }
    }

    /// Sources the runner executables of the current release in `runners`,
    /// a server release's `runners/`, for the targets of the systems of the
    /// paired devices `control` lists, one after another, so that an update
    /// never waits on the release's origin (`crates/backend-http/src/install.rs`). The device records name a system, not a target, so each
    /// of the system's targets is sourced. A failure is logged: a runner's
    /// update sources its executable again.
    pub async fn source_paired_runners(
        &self,
        runners: &Path,
        control: &ControlService,
        cancel: &CancellationToken,
    ) {
        let release = match read_runner_release(&runners.join("manifest.json"), RunnerRelease::decode).await {
            Ok(Some(release)) => release,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!("the runner release to source cannot be read: {error}");
                return;
            }
        };
        let platforms: Vec<RunnerPlatform> = match control.paired_systems().await {
            Ok(systems) => systems.into_iter().map(|system| system.platform).collect(),
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

/// The targets a Host of `platform` may run: the one of `arch`, the
/// processor architecture as Rust names it, when it is known, and else each
/// of the platform's.
pub fn system_targets(
    platform: RunnerPlatform,
    arch: Option<&str>,
) -> impl Iterator<Item = &'static str> {
    TARGETS.iter().copied().filter(move |target| {
        runs(platform, target)
            && arch.is_none_or(|arch| target.split('-').next() == Some(arch))
    })
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
