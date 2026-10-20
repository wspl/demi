//! What the native store keeps (`crates/backend-runners/src/publication.rs`): as the backend starts, once its release's
//! packages are published, it records its release under
//! `native/releases/<version>.json`, naming each package's version and each
//! runner executable's SHA-256. It keeps its own record and the newest other
//! one, the release that ran before it, and removes every other record, then
//! every mapping no kept record names, every descriptor no kept mapping
//! names, and every blob that neither a kept descriptor nor a kept record
//! names. Removal runs in that order, so an interrupted one never leaves a
//! kept name without its object. Every mapping and descriptor a kept record
//! names is read first; when one is missing, nothing is removed.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use bytes::Bytes;
use demi_command_protocol::PackageDescriptor;
use demi_runner_protocol::release::RunnerRelease;
use futures_util::{StreamExt as _, TryStreamExt as _};
use object_store::path::Path;
use object_store::{
    Attribute, GetOptions, ObjectMeta, ObjectStore, ObjectStoreExt as _, PutMode, PutOptions,
    PutPayload,
};
use percent_encoding::{percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::publication::{COMPONENT, PREFIX, PublicationError, SHA256, blob, mapping};

/// A release's record: what of the store it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseRecord {
    /// Each package's version, by package id.
    packages: BTreeMap<String, String>,
    /// The SHA-256 of each runner executable of the release.
    runners: BTreeSet<String>,
}

/// What a pruning removed, by kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pruned {
    pub records: usize,
    pub mappings: usize,
    pub descriptors: usize,
    pub blobs: usize,
}

/// Records release `version`, whose packages are `descriptors` and whose
/// runner release is `runners`, then removes what neither its record nor
/// the newest other one names. `cancel` stops it between steps.
pub async fn keep_releases(
    store: &dyn ObjectStore,
    version: &str,
    descriptors: &[PackageDescriptor],
    runners: Option<&RunnerRelease>,
    cancel: &CancellationToken,
) -> Result<Pruned, PublicationError> {
    let own = ReleaseRecord {
        packages: descriptors
            .iter()
            .map(|descriptor| (descriptor.id.clone(), descriptor.version.clone()))
            .collect(),
        runners: runners
            .into_iter()
            .flat_map(|release| release.targets.values())
            .map(|artifact| artifact.sha256.clone())
            .collect(),
    };
    let own_key = record_key(version)?;
    // Written again at every start, so its time is when this release last
    // started, which the next release's start compares.
    let body = serde_json::to_vec(&own).expect("a release record encodes as JSON");
    let options = PutOptions {
        mode: PutMode::Overwrite,
        ..PutOptions::default()
    };
    cancellable(cancel, store.put_opts(&own_key, PutPayload::from(body), options)).await?;

    let records = listed(store, "releases", cancel).await?;
    let mut others = Vec::new();
    for meta in records.iter().filter(|meta| meta.location != own_key) {
        others.push((meta.last_modified, record_version(&meta.location)?, &meta.location));
    }
    // The newest by the time written; two of one millisecond by version.
    let previous = others.into_iter().max().map(|(_, _, key)| key.clone());
    let mut kept = vec![own];
    if let Some(previous) = &previous {
        kept.push(read_record(store, previous, cancel).await?);
    }

    // What the kept records name, each read where it must be. One that is
    // missing means the store is damaged: what a kept release still has is
    // then not told apart from what no release names, so nothing goes.
    let mut kept_mappings = HashSet::new();
    let mut kept_descriptors = HashSet::new();
    let mut kept_blobs = HashSet::new();
    let mut missing = Vec::new();
    for record in &kept {
        for sha256 in &record.runners {
            kept_blobs.insert(blob(sha256)?);
        }
        for (id, version) in &record.packages {
            let claim = mapping(id, version)?;
            let Some(digest) = named_sha256(store, &claim, cancel).await? else {
                missing.push(claim.to_string());
                continue;
            };
            kept_mappings.insert(claim);
            let named = descriptor_key(&digest)?;
            let Some(descriptor) = read_descriptor(store, &named, cancel).await? else {
                missing.push(named.to_string());
                continue;
            };
            kept_descriptors.insert(named);
            for artifact in descriptor.targets.values() {
                kept_blobs.insert(blob(&artifact.sha256)?);
            }
        }
    }
    if !missing.is_empty() {
        return Err(PublicationError::Missing(missing.join(", ")));
    }

    // Records first, blobs last: a removal cut short leaves no kept name
    // without its object.
    let mut pruned = Pruned::default();
    let kept_records: HashSet<&Path> = [Some(&own_key), previous.as_ref()].into_iter().flatten().collect();
    pruned.records = remove(
        store,
        records.iter().map(|meta| &meta.location).filter(|key| !kept_records.contains(key)),
        cancel,
    )
    .await?;
    for (kind, kept, count) in [
        ("packages", &kept_mappings, &mut pruned.mappings),
        ("descriptors", &kept_descriptors, &mut pruned.descriptors),
        ("blobs", &kept_blobs, &mut pruned.blobs),
    ] {
        let listed = listed(store, kind, cancel).await?;
        *count = remove(
            store,
            listed.iter().map(|meta| &meta.location).filter(|key| !kept.contains(*key)),
            cancel,
        )
        .await?;
    }
    Ok(pruned)
}

/// `native/releases/<version>.json`, the version percent-encoded as a
/// mapping's is.
fn record_key(version: &str) -> Result<Path, PublicationError> {
    let version = utf8_percent_encode(version, COMPONENT);
    crate::publication::object(format!("{PREFIX}/releases/{version}.json"))
}

/// The server version a record's key names, which orders two records
/// written in the same millisecond.
fn record_version(key: &Path) -> Result<semver::Version, PublicationError> {
    let corrupt = |reason: String| PublicationError::Corrupt {
        key: key.to_string(),
        reason,
    };
    let name = key
        .filename()
        .and_then(|name| name.strip_suffix(".json"))
        .ok_or_else(|| corrupt("it is no release record".into()))?;
    let version = percent_decode_str(name)
        .decode_utf8()
        .map_err(|error| corrupt(error.to_string()))?;
    semver::Version::parse(&version).map_err(|error| corrupt(error.to_string()))
}

fn descriptor_key(digest: &str) -> Result<Path, PublicationError> {
    crate::publication::object(format!("{PREFIX}/descriptors/{digest}.json"))
}

async fn cancellable<T>(
    cancel: &CancellationToken,
    work: impl Future<Output = object_store::Result<T>>,
) -> Result<T, PublicationError> {
    tokio::select! {
        () = cancel.cancelled() => Err(PublicationError::Cancelled),
        done = work => Ok(done?),
    }
}

/// Every object under `native/<kind>/`.
async fn listed(
    store: &dyn ObjectStore,
    kind: &str,
    cancel: &CancellationToken,
) -> Result<Vec<ObjectMeta>, PublicationError> {
    let prefix = crate::publication::object(format!("{PREFIX}/{kind}"))?;
    cancellable(cancel, store.list(Some(&prefix)).try_collect()).await
}

async fn read(
    store: &dyn ObjectStore,
    key: &Path,
    cancel: &CancellationToken,
) -> Result<Bytes, PublicationError> {
    cancellable(cancel, async { store.get(key).await?.bytes().await }).await
}

/// The record at `key`; one that does not decode stops the pruning, which
/// cannot tell what it names.
async fn read_record(
    store: &dyn ObjectStore,
    key: &Path,
    cancel: &CancellationToken,
) -> Result<ReleaseRecord, PublicationError> {
    let bytes = read(store, key, cancel).await?;
    serde_json::from_slice(&bytes).map_err(|error| PublicationError::Corrupt {
        key: key.to_string(),
        reason: error.to_string(),
    })
}

/// The descriptor at `key`; none when the store lacks it.
async fn read_descriptor(
    store: &dyn ObjectStore,
    key: &Path,
    cancel: &CancellationToken,
) -> Result<Option<PackageDescriptor>, PublicationError> {
    let bytes = match read(store, key, cancel).await {
        Err(PublicationError::Store(object_store::Error::NotFound { .. })) => return Ok(None),
        read => read?,
    };
    let corrupt = |reason: String| PublicationError::Corrupt {
        key: key.to_string(),
        reason,
    };
    let value = serde_json::from_slice(&bytes).map_err(|error| corrupt(error.to_string()))?;
    PackageDescriptor::parse(value)
        .map(Some)
        .map_err(|error| corrupt(error.to_string()))
}

/// The SHA-256 a mapping's metadata names, its descriptor's digest; none
/// when the store lacks the mapping.
async fn named_sha256(
    store: &dyn ObjectStore,
    key: &Path,
    cancel: &CancellationToken,
) -> Result<Option<String>, PublicationError> {
    let head = GetOptions {
        head: true,
        ..GetOptions::default()
    };
    let found = match cancellable(cancel, store.get_opts(key, head)).await {
        Err(PublicationError::Store(object_store::Error::NotFound { .. })) => return Ok(None),
        found => found?,
    };
    found
        .attributes
        .get(&Attribute::Metadata(SHA256.into()))
        .map(|value| Some(value.as_ref().to_owned()))
        .ok_or_else(|| PublicationError::Corrupt {
            key: key.to_string(),
            reason: "it names no SHA-256".into(),
        })
}

/// Removes `keys`; answers how many.
async fn remove<'a>(
    store: &dyn ObjectStore,
    keys: impl Iterator<Item = &'a Path>,
    cancel: &CancellationToken,
) -> Result<usize, PublicationError> {
    let keys: Vec<object_store::Result<Path>> = keys.cloned().map(Ok).collect();
    if keys.is_empty() {
        return Ok(0);
    }
    let removed = store.delete_stream(futures_util::stream::iter(keys).boxed());
    let removed: Vec<Path> = cancellable(cancel, removed.try_collect()).await?;
    Ok(removed.len())
}
