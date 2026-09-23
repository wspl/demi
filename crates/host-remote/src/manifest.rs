//! Building manifests from a command set (`commands.md` § Dispatch the same
//! declaration on each surface): the backend's catalog of native packages,
//! fixed at startup, pins each declared native command to its package's
//! descriptor, and a job carries the resulting selection.

use std::rc::Rc;

use demi_command_service::protocol::{ArtifactLocation, PackageArtifact, PackageDescriptor};
use demi_runner_protocol::manifest::{Manifest, ManifestError};
use demi_shell::CommandSet;
use futures_util::future::LocalBoxFuture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// Where a runner downloads a package's executable: a signed URL the backend
/// makes, or a path on the runner's machine in tests. `cancel` ends when the
/// work that asked no longer runs.
pub trait ArtifactResolver {
    fn resolve(
        &self,
        artifact: &PackageArtifact,
        target: &str,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>>;
}

/// The native packages commands bind to, and where their executables are.
#[derive(Clone)]
pub struct CommandCatalog {
    packages: Rc<[PackageDescriptor]>,
    resolver: Rc<dyn ArtifactResolver>,
}

impl CommandCatalog {
    /// A catalog whose descriptors are valid and whose package ids are
    /// unique.
    pub fn new(
        packages: Vec<PackageDescriptor>,
        resolver: Rc<dyn ArtifactResolver>,
    ) -> Result<Self, ManifestError> {
        Manifest::build([], packages.iter().cloned())?;
        Ok(Self {
            packages: packages.into(),
            resolver,
        })
    }

    /// The manifest of `commands`: each native command pinned to the
    /// descriptor of its package in this catalog.
    pub fn select(&self, commands: &CommandSet) -> Result<CommandSelection, ManifestError> {
        let manifest = Manifest::build(
            commands.declarations().cloned(),
            self.packages.iter().cloned(),
        )?;
        let wire = serde_json::to_value(&manifest)?;
        Ok(CommandSelection(Rc::new(Selection {
            manifest,
            wire,
            resolver: self.resolver.clone(),
        })))
    }
}

/// A manifest, and the resolver of its packages' executables: what a job
/// pins.
#[derive(Clone)]
pub struct CommandSelection(Rc<Selection>);

struct Selection {
    manifest: Manifest,
    /// The manifest as the wire carries it, written once.
    wire: Value,
    resolver: Rc<dyn ArtifactResolver>,
}

impl CommandSelection {
    pub fn manifest(&self) -> &Manifest {
        &self.0.manifest
    }

    pub fn hash(&self) -> &str {
        &self.0.manifest.hash
    }

    pub(crate) fn wire(&self) -> &Value {
        &self.0.wire
    }

    pub(crate) fn packages(&self) -> Vec<PackageDescriptor> {
        self.0.manifest.packages.values().cloned().collect()
    }

    pub(crate) fn resolver(&self) -> Rc<dyn ArtifactResolver> {
        self.0.resolver.clone()
    }
}
