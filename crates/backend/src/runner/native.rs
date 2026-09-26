//! The native command packages the conversations' commands bind to
//! (`native-runtime.md` § Publish artifacts before enabling commands,
//! § Backend deployment configuration): the descriptors of the loaded
//! releases, and where a runner downloads each package's executables, from
//! object storage or, for a development store, from this backend. The
//! artifact module makes the catalog at startup from `DEMI_NATIVE_CONFIG`.
//! Each shard thread builds its own `CommandCatalog` from it, since a
//! catalog's artifact resolver lives on one thread.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use demi_command_service::protocol::{ArtifactLocation, PackageArtifact, PackageDescriptor};
use demi_host_remote::{ArtifactResolver, CommandCatalog};
use demi_runner_protocol::manifest::ManifestError;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use super::local_store::{LocalArtifacts, ServedArtifacts};
use super::publication::SignedArtifacts;
use crate::backend::PublicUrl;

/// The loaded native packages, as each shard's catalog is built from them.
#[derive(Clone)]
pub struct NativeCatalog {
    packages: Vec<PackageDescriptor>,
    store: Store,
}

/// Where runners download the packages' executables.
#[derive(Clone)]
pub(crate) enum Store {
    /// No package, so nothing to download.
    Unpublished,
    /// Object storage, through a URL signed for each request.
    Signed(SignedArtifacts),
    /// A development store: this backend serves them itself.
    Local(Arc<LocalArtifacts>),
}

impl NativeCatalog {
    /// The catalog of `packages`, whose executables `store` holds. It is
    /// refused when a descriptor is invalid or two packages share an id.
    pub(crate) fn new(packages: Vec<PackageDescriptor>, store: Store) -> Result<Self, ManifestError> {
        CommandCatalog::new(packages.clone(), Rc::new(Unpublished))?;
        Ok(Self { packages, store })
    }

    /// No package: a command set that declares a native command selects no
    /// manifest, so a node whose commands include one gets no shell. A
    /// test's backend runs on it unless it loads releases; the product's
    /// start loads the releases `DEMI_NATIVE_CONFIG` names instead
    /// (`publish_native`).
    pub(crate) fn unpublished() -> Self {
        Self {
            packages: Vec::new(),
            store: Store::Unpublished,
        }
    }

    /// The calling thread's catalog; a development store's downloads are on
    /// `backend`.
    pub(crate) fn catalog(&self, backend: &PublicUrl) -> CommandCatalog {
        CommandCatalog::new(self.packages.clone(), self.resolver(backend))
            .expect("the packages were checked when the catalog was made")
    }

    /// The calling thread's resolver of the packages' executables, for the
    /// work that binds a package without a manifest: a user stream or a
    /// one-shot user call. A development store's downloads are on
    /// `backend`.
    pub(crate) fn resolver(&self, backend: &PublicUrl) -> Rc<dyn ArtifactResolver> {
        match &self.store {
            Store::Unpublished => Rc::new(Unpublished),
            Store::Signed(signed) => Rc::new(signed.clone()),
            Store::Local(artifacts) => Rc::new(ServedArtifacts {
                artifacts: artifacts.clone(),
                backend: backend.clone(),
            }),
        }
    }

    /// The file of the executable whose SHA-256 is `sha256`, when a
    /// development store serves it.
    pub(crate) fn local_file(&self, sha256: &str) -> Option<&Path> {
        match &self.store {
            Store::Local(artifacts) => artifacts.file(sha256),
            Store::Unpublished | Store::Signed(_) => None,
        }
    }

    /// The loaded package `id`.
    pub(crate) fn package(&self, id: &str) -> Option<&PackageDescriptor> {
        self.packages.iter().find(|descriptor| descriptor.id == id)
    }

    /// Whether `package` serves every one of `operations`.
    pub(crate) fn serves(&self, package: &str, operations: &[&str]) -> bool {
        self.package(package).is_some_and(|descriptor| {
            operations
                .iter()
                .all(|operation| descriptor.operations.iter().any(|served| served == operation))
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
        let message = format!("no native package is published for {target}");
        Box::pin(async move { Err(message) })
    }
}
