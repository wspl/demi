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

use std::path::{Path as FilePath, PathBuf};

use bytes::Bytes;
use demi_core_blobs::store::Objects;
use demi_command_protocol::{PackageArtifact, PackageDescriptor};
use demi_runner_protocol::release::RunnerRelease;
use object_store::path::Path;
use object_store::{
    Attribute, Attributes, GetOptions, ObjectStore, PutMode, PutOptions, PutPayload,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio_util::sync::CancellationToken;

use crate::native::{Artifacts, NativeCatalog};
use crate::retention::keep_releases;
use crate::sourcing::Sourcing;
use demi_core_hosts::packages::{ReleaseError, ReleasePackages};

/// The key prefix of every command package object in the store
/// (`crates/core-blobs/src/store.rs`).
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

impl From<ReleaseError> for PublicationError {
    fn from(error: ReleaseError) -> Self {
        match error {
            ReleaseError::Commands { directory, reason } => Self::Commands { directory, reason },
            ReleaseError::Release { directory, reason } => Self::Release { directory, reason },
        }
    }
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
    let ReleasePackages { descriptors, executables } = ReleasePackages::read(commands).await?;
    publish(&descriptors, &*objects.store, cancel).await?;
    match keep_releases(&*objects.store, version, &descriptors, runners, cancel).await {
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
    let artifacts = Artifacts::new(sourcing, objects.signer.clone(), executables);
    tracing::info!(
        event = "command_packages.published",
        packages = descriptors.len(),
        "the command packages are published"
    );
    NativeCatalog::new(descriptors, artifacts).map_err(|error| PublicationError::Commands {
        directory: commands.to_owned(),
        reason: error.to_string(),
    })
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
