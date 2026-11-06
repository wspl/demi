//! The command packages a core's runners run, which whoever runs the core
//! gives: a server publishes its release's, a program carries its own.

use std::rc::Rc;

use demi_command_protocol::PackageDescriptor;
use demi_runner_client::{ArtifactResolver, CommandCatalog};

/// The packages, and where a runner fetches their executables.
pub trait Packages: Send + Sync {
    /// The package `id`.
    fn package(&self, id: &str) -> Option<&PackageDescriptor>;

    /// The calling thread's resolver of the packages' executables, for the
    /// work that binds a package without a manifest: a user stream or a
    /// one-shot user call.
    fn resolver(&self) -> Rc<dyn ArtifactResolver>;

    /// The calling thread's catalog of the packages.
    fn catalog(&self) -> CommandCatalog;

    /// Whether `package` serves every one of `operations`.
    fn serves(&self, package: &str, operations: &[&str]) -> bool {
        self.package(package).is_some_and(|descriptor| {
            operations
                .iter()
                .all(|operation| descriptor.operations.iter().any(|served| served == operation))
        })
    }
}
