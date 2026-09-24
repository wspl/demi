//! The native command packages the conversations' commands bind to
//! (`native-runtime.md` § Publish artifacts before enabling commands,
//! § Backend deployment configuration): the descriptors of the published
//! releases, and where a runner downloads each package's executables. The
//! artifact module makes the catalog at startup from `DEMI_NATIVE_CONFIG`.
//! Each shard thread builds its own `CommandCatalog` from it, since a
//! catalog's artifact resolver lives on one thread.

use std::rc::Rc;
use std::sync::Arc;

use demi_command_service::protocol::{ArtifactLocation, PackageArtifact, PackageDescriptor};
use demi_host_remote::{ArtifactResolver, CommandCatalog};
use demi_runner_protocol::manifest::ManifestError;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

/// Makes the resolver of the packages' executables for the calling thread.
type Resolvers = Arc<dyn Fn() -> Rc<dyn ArtifactResolver> + Send + Sync>;

/// The published native packages, as each shard's catalog is built from
/// them.
#[derive(Clone)]
pub struct NativeCatalog {
    packages: Vec<PackageDescriptor>,
    resolvers: Resolvers,
}

impl NativeCatalog {
    /// The catalog of `packages`, whose executables the resolvers `resolver`
    /// makes find. It is refused when a descriptor is invalid or two
    /// packages share an id.
    pub fn new(
        packages: Vec<PackageDescriptor>,
        resolver: impl Fn() -> Rc<dyn ArtifactResolver> + Send + Sync + 'static,
    ) -> Result<Self, ManifestError> {
        CommandCatalog::new(packages.clone(), resolver())?;
        Ok(Self {
            packages,
            resolvers: Arc::new(resolver),
        })
    }

    /// No published package: a command set that declares a native command
    /// selects no manifest, so a node whose commands include one gets no
    /// shell. Interim: the product runs on this until the artifact module
    /// publishes the releases `DEMI_NATIVE_CONFIG` names and makes the
    /// catalog from them.
    pub(crate) fn unpublished() -> Self {
        Self {
            packages: Vec::new(),
            resolvers: Arc::new(|| Rc::new(Unpublished) as Rc<dyn ArtifactResolver>),
        }
    }

    /// The calling thread's catalog.
    pub(crate) fn catalog(&self) -> CommandCatalog {
        CommandCatalog::new(self.packages.clone(), (self.resolvers)())
            .expect("the packages were checked when the catalog was made")
    }

    /// Whether `package` serves every one of `operations`.
    pub(crate) fn serves(&self, package: &str, operations: &[&str]) -> bool {
        self.packages.iter().any(|descriptor| {
            descriptor.id == package
                && operations
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
