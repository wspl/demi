//! The command packages a core's runners run, which whoever runs the core
//! gives: a server publishes its release's, a program's process serves its
//! release's.
//!
//! A release's `commands/` holds one directory per package, named as its
//! program, with the package's `descriptor.json`; its `files/` holds each
//! executable's compressed copy, which a runner downloads in the content
//! coding.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use demi_command_protocol::{PackageArtifact, PackageDescriptor};
use demi_runner_client::{ArtifactResolver, CommandCatalog};
use demi_runner_protocol::release::compressed_file;

/// Where a core's own artifacts download from, at its root: a server's
/// local store, a process's release files.
pub const ARTIFACT_ROUTE: &str = "/native-artifacts";

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

/// A release's command packages: each one's descriptor, and each
/// executable's file in the release's `files/`.
#[derive(Debug, Clone, Default)]
pub struct ReleasePackages {
    pub descriptors: Vec<PackageDescriptor>,
    /// Each package executable, by SHA-256.
    pub executables: HashMap<String, ReleaseArtifact>,
}

/// An artifact of the release: the file of the release's files that holds
/// it, what it stands for, and whether the file is its compressed copy,
/// which is served as it is, in the content coding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseArtifact {
    pub file: String,
    pub artifact: PackageArtifact,
    pub encoded: bool,
}

/// Why a release's command packages cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("the command packages in {} cannot be read: {reason}", directory.display())]
    Commands { directory: PathBuf, reason: String },
    #[error("the command package in {} is not valid: {reason}", directory.display())]
    Release { directory: PathBuf, reason: String },
}

impl ReleasePackages {
    /// Reads the packages in `commands`, a release's `commands/`, in the
    /// order of their programs' names. An empty directory has none; a
    /// missing one is an error, since only the empty directory means none.
    /// A descriptor must carry a target, and two of one package are refused.
    pub async fn read(commands: &Path) -> Result<Self, ReleaseError> {
        let mut packages = Self::default();
        let mut ids = HashSet::new();
        for (directory, executable) in releases(commands).await? {
            let descriptor = descriptor(&directory).await?;
            if !ids.insert(descriptor.id.clone()) {
                return Err(ReleaseError::Release {
                    directory,
                    reason: "the package is released twice".into(),
                });
            }
            for (target, artifact) in &descriptor.targets {
                let wanted = ReleaseArtifact {
                    file: compressed_file(&executable, target),
                    artifact: artifact.clone(),
                    encoded: true,
                };
                packages.executables.entry(artifact.sha256.clone()).or_insert(wanted);
            }
            packages.descriptors.push(descriptor);
        }
        Ok(packages)
    }
}

/// The package directories in `commands`, each with its program's name,
/// in the order of the names.
async fn releases(commands: &Path) -> Result<Vec<(PathBuf, String)>, ReleaseError> {
    let unreadable = |reason: String| ReleaseError::Commands {
        directory: commands.to_owned(),
        reason,
    };
    let mut entries = tokio::fs::read_dir(commands)
        .await
        .map_err(|error| unreadable(error.to_string()))?;
    let mut releases = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| unreadable(error.to_string()))?
    {
        let name = entry.file_name();
        let executable = name
            .to_str()
            .filter(|name| {
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            })
            .ok_or_else(|| unreadable(format!("{} is no executable's name", name.display())))?;
        releases.push((entry.path(), executable.to_owned()));
    }
    releases.sort_by(|first, second| first.1.cmp(&second.1));
    Ok(releases)
}

/// The descriptor in `directory`, which must carry a target.
async fn descriptor(directory: &Path) -> Result<PackageDescriptor, ReleaseError> {
    let refused = |reason: String| ReleaseError::Release {
        directory: directory.to_owned(),
        reason,
    };
    let text = tokio::fs::read(directory.join("descriptor.json"))
        .await
        .map_err(|error| refused(format!("descriptor.json: {error}")))?;
    let value = serde_json::from_slice(&text)
        .map_err(|error| refused(format!("descriptor.json: {error}")))?;
    let descriptor = PackageDescriptor::parse(value)
        .map_err(|error| refused(format!("descriptor.json: {error}")))?;
    if descriptor.targets.is_empty() {
        return Err(refused("it carries no target".into()));
    }
    Ok(descriptor)
}
