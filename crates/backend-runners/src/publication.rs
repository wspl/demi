//! Native package publication: before the backend accepts requests, every
//! command package release in its server release's `commands/` is read and
//! checked.
//!
//! 1. Its descriptor and then its immutable package-and-version mapping are
//!    published into the deployment's one object store, local or S3 alike.
//! 2. Only then does the catalog serve.
//!
//! No executable is stored here: each enters the store the first time something
//! needs it ([`crate::sourcing`]).
//!
//! Every object carries the SHA-256 and size of what it stands for, the
//! executable's for an encoded one. A write never replaces an object: finding
//! one in place is success when those are the ones published, and a refusal
//! otherwise.

use std::collections::{HashMap, HashSet};
use std::path::{Path as FilePath, PathBuf};

use bytes::Bytes;
use demi_backend_blobs::store::Objects;
use demi_command_protocol::{PackageArtifact, PackageDescriptor};
use demi_runner_protocol::release::{RunnerRelease, compressed_file};
use object_store::path::Path;
use object_store::{
    Attribute, Attributes, GetOptions, ObjectStore, PutMode, PutOptions, PutPayload,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio_util::sync::CancellationToken;

use crate::native::{Artifacts, NativeCatalog};
use crate::retention::keep_releases;
use crate::sourcing::{ReleaseArtifact, Sourcing};

/// The key prefix of every command package object in the store
/// (`crates/backend-blobs/src/store.rs`).
pub const PREFIX: &str = "native";

/// The metadata keys of the SHA-256 and the size of what an object stands
/// for.
pub(crate) const SHA256: &str = "sha256";
const SIZE: &str = "size";

/// What a package key leaves unencoded in a version: what a URI component
/// may carry as it is.
pub(crate) const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// One release directory of `commands/`, named as its program, which holds
/// its `descriptor.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Release {
    directory: PathBuf,
    executable: String,
}

/// Why publication stopped the start.
#[derive(Debug, thiserror::Error)]
pub enum PublicationError {
    #[error("the command packages in {} cannot be read: {reason}", directory.display())]
    Commands { directory: PathBuf, reason: String },
    #[error("the native release in {} cannot be published: {reason}", directory.display())]
    Release { directory: PathBuf, reason: String },
    #[error("an immutable native artifact object differs from the one in place: {0}")]
    Conflict(String),
    #[error("the native store's object {key} is not valid: {reason}")]
    Corrupt { key: String, reason: String },
    #[error("the native store lacks what a kept release names: {0}")]
    Missing(String),
    #[error("the object store failed: {0}")]
    Store(#[from] object_store::Error),
    #[error("native artifact publication was interrupted")]
    Cancelled,
}

/// Publishes the releases in `commands`, a server release's `commands/`,
/// into `objects`, and makes the catalog of them, which the conversations'
/// commands bind to and whose executables come from `files`, the root's
/// `files/`. Then it records server release `version`, whose runner release is
/// `runners`, and removes the packages of older releases
/// ([`crate::retention`]). The backend accepts requests only once this
/// completed; `cancel` interrupts it.
pub async fn publish_native(
    commands: &FilePath,
    files: PathBuf,
    version: &str,
    runners: Option<&RunnerRelease>,
    objects: &Objects,
    cancel: &CancellationToken,
) -> Result<NativeCatalog, PublicationError> {
    let (releases, verified) = read_releases(commands).await?;
    publish(&verified, &*objects.store, cancel).await?;
    match keep_releases(&*objects.store, version, &verified, runners, cancel).await {
        Ok(pruned) => tracing::info!(
            event = "native_store.pruned",
            records = pruned.records,
            mappings = pruned.mappings,
            descriptors = pruned.descriptors,
            blobs = pruned.blobs,
            "the native store keeps this release and the one before"
        ),
        Err(PublicationError::Cancelled) => return Err(PublicationError::Cancelled),
        // What stays is only stored, never served wrongly; the next start
        // prunes again.
        Err(error) => tracing::warn!(event = "native_store.prune_failed", %error, "the native store's older releases stay"),
    }
    let sourcing = Sourcing::new(objects.store.clone(), files);
    let artifacts = Artifacts::new(sourcing, objects.signer.clone(), executables(&releases, &verified));
    tracing::info!(
        event = "command_packages.published",
        packages = verified.len(),
        "the command packages are published"
    );
    NativeCatalog::new(verified, artifacts).map_err(|error| PublicationError::Commands {
        directory: commands.to_owned(),
        reason: error.to_string(),
    })
}

/// The releases in `commands` and their verified descriptors, in the same
/// order.
pub(crate) async fn read_releases(
    commands: &FilePath,
) -> Result<(Vec<Release>, Vec<PackageDescriptor>), PublicationError> {
    let releases = releases(commands).await?;
    let verified = verify_all(&releases).await?;
    Ok((releases, verified))
}

/// Each package executable of `releases`, whose descriptors are `verified`,
/// by SHA-256, as the release's `files/` holds its compressed copy.
pub(crate) fn executables(
    releases: &[Release],
    verified: &[PackageDescriptor],
) -> HashMap<String, ReleaseArtifact> {
    let mut executables = HashMap::new();
    for (release, descriptor) in releases.iter().zip(verified) {
        for (target, artifact) in &descriptor.targets {
            let wanted = ReleaseArtifact {
                file: compressed_file(&release.executable, target),
                artifact: artifact.clone(),
                encoded: true,
            };
            executables.entry(artifact.sha256.clone()).or_insert(wanted);
        }
    }
    executables
}

/// The releases in `commands`, one per directory, in the order of their
/// names. An empty directory has none; a missing one is an error, since
/// only the empty directory means none.
async fn releases(commands: &FilePath) -> Result<Vec<Release>, PublicationError> {
    let unreadable = |reason: String| PublicationError::Commands {
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
        releases.push(Release {
            directory: entry.path(),
            executable: executable.to_owned(),
        });
    }
    releases.sort_by(|first, second| first.executable.cmp(&second.executable));
    Ok(releases)
}

/// Publishes each descriptor's canonical JSON and then its package and
/// version's mapping. `cancel` stops it.
async fn publish(
    descriptors: &[PackageDescriptor],
    store: &dyn ObjectStore,
    cancel: &CancellationToken,
) -> Result<(), PublicationError> {
    for descriptor in descriptors {
        let body = serde_json_canonicalizer::to_vec(descriptor).map_err(|error| {
            PublicationError::Release {
                directory: PathBuf::from(&descriptor.id),
                reason: error.to_string(),
            }
        })?;
        let digest = descriptor
            .digest()
            .map_err(|error| PublicationError::Release {
                directory: PathBuf::from(&descriptor.id),
                reason: error.to_string(),
            })?;
        let artifact = PackageArtifact {
            sha256: digest,
            size: body.len() as u64,
        };
        let body = Bytes::from(body);
        let named = object(format!("{PREFIX}/descriptors/{}.json", artifact.sha256))?;
        put_immutable(store, &named, body.clone(), &artifact, None, cancel).await?;
        // The package and version's meaning, published last: a conflicting
        // descriptor fails instead of changing what the version names.
        let claim = mapping(&descriptor.id, &descriptor.version)?;
        put_immutable(store, &claim, body, &artifact, None, cancel).await?;
    }
    Ok(())
}

/// Reads every release's descriptor. Two releases of one package are
/// refused.
async fn verify_all(releases: &[Release]) -> Result<Vec<PackageDescriptor>, PublicationError> {
    let mut verified = Vec::with_capacity(releases.len());
    let mut ids = HashSet::new();
    for release in releases {
        let descriptor = verify(release).await?;
        if !ids.insert(descriptor.id.clone()) {
            return Err(PublicationError::Release {
                directory: release.directory.clone(),
                reason: "the package is released twice".into(),
            });
        }
        verified.push(descriptor);
    }
    Ok(verified)
}

/// Reads a release's descriptor, which must carry a target.
async fn verify(release: &Release) -> Result<PackageDescriptor, PublicationError> {
    let refused = |reason: String| PublicationError::Release {
        directory: release.directory.clone(),
        reason,
    };
    let text = tokio::fs::read(release.directory.join("descriptor.json"))
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

/// The mapping of package `id`'s `version`,
/// `native/packages/<id>/<version>.json`, the version percent-encoded.
pub(crate) fn mapping(id: &str, version: &str) -> Result<Path, PublicationError> {
    let version = utf8_percent_encode(version, COMPONENT);
    object(format!("{PREFIX}/packages/{id}/{version}.json"))
}

/// The object at `key`, which is taken as it is: a version's percent
/// signs are part of its key.
pub(crate) fn object(key: String) -> Result<Path, PublicationError> {
    Path::parse(&key).map_err(|error| PublicationError::Release {
        directory: PathBuf::from(&key),
        reason: format!("no object key: {error}"),
    })
}

/// A content-addressed artifact's key.
pub fn blob(sha256: &str) -> Result<Path, PublicationError> {
    object(format!("{PREFIX}/blobs/{sha256}"))
}

/// Creates the object at `path`, which stands for `artifact`, from `bytes`
/// in `coding`, unless one is there; one in place is success when it stands
/// for `artifact`, and a conflict otherwise.
pub(crate) async fn put_immutable(
    store: &dyn ObjectStore,
    path: &Path,
    bytes: Bytes,
    artifact: &PackageArtifact,
    coding: Option<&'static str>,
    cancel: &CancellationToken,
) -> Result<(), PublicationError> {
    let mut attributes = Attributes::new();
    attributes.insert(
        Attribute::Metadata(SHA256.into()),
        artifact.sha256.clone().into(),
    );
    attributes.insert(
        Attribute::Metadata(SIZE.into()),
        artifact.size.to_string().into(),
    );
    attributes.insert(Attribute::ContentType, "application/octet-stream".into());
    if let Some(coding) = coding {
        attributes.insert(Attribute::ContentEncoding, coding.into());
    }
    let options = PutOptions {
        mode: PutMode::Create,
        attributes,
        ..PutOptions::default()
    };
    let put = tokio::select! {
        () = cancel.cancelled() => return Err(PublicationError::Cancelled),
        put = store.put_opts(path, PutPayload::from_bytes(bytes), options) => put,
    };
    match put {
        Ok(_) => Ok(()),
        Err(object_store::Error::AlreadyExists { .. }) => {
            in_place(store, path, artifact, cancel).await.map(drop)
        }
        Err(error) => Err(error.into()),
    }
}

/// Whether an object at `path` stands for `artifact`: none is `false`, one
/// whose published SHA-256 and size are the artifact's is `true`, and any
/// other is a conflict.
pub(crate) async fn in_place(
    store: &dyn ObjectStore,
    path: &Path,
    artifact: &PackageArtifact,
    cancel: &CancellationToken,
) -> Result<bool, PublicationError> {
    let head = GetOptions {
        head: true,
        ..GetOptions::default()
    };
    let found = tokio::select! {
        () = cancel.cancelled() => return Err(PublicationError::Cancelled),
        found = store.get_opts(path, head) => found,
    };
    let existing = match found {
        Ok(existing) => existing,
        Err(object_store::Error::NotFound { .. }) => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let published = |key: &str| {
        existing
            .attributes
            .get(&Attribute::Metadata(key.to_owned().into()))
            .map(|value| value.as_ref().to_owned())
    };
    let same = published(SHA256).as_deref() == Some(artifact.sha256.as_str())
        && published(SIZE) == Some(artifact.size.to_string());
    if !same {
        return Err(PublicationError::Conflict(path.to_string()));
    }
    Ok(true)
}
