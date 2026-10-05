//! Native artifact publication (`native-runtime.md` § Publish artifacts
//! before enabling commands, § Backend deployment configuration): before the
//! backend accepts requests, every command package release in its server
//! release's `commands/` is verified whole (its descriptor, and each
//! target's executable by size and SHA-256), and published into the
//! deployment's one object store, local or S3 alike: its executables are
//! stored once each as content-addressed objects, as the compressed copy its
//! release carries once that copy is checked to decode to the executable,
//! its descriptor and then its immutable package-and-version mapping are
//! published, and only then does the catalog serve. Every object carries the
//! SHA-256 and size of what it stands for, the executable's for an encoded
//! one. A write never replaces an object: finding one in place is success when
//! those are the ones published, and a refusal otherwise. Runners download an
//! executable from a URL signed for five minutes when the store is S3, and
//! from the backend itself when it is local (`local_store`).

use std::collections::{HashMap, HashSet};
use std::path::{Path as FilePath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use bytes::Bytes;
use demi_backend_blobs::store::Objects;
use demi_backend_remote_host::ArtifactResolver;
use demi_command_protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact, PackageDescriptor};
use futures_util::TryStreamExt as _;
use futures_util::future::LocalBoxFuture;
use object_store::path::Path;
use object_store::signer::Signer;
use object_store::{
    Attribute, Attributes, GetOptions, ObjectStore, PutMode, PutOptions, PutPayload,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio_util::sync::CancellationToken;

use crate::local_store::LocalArtifacts;
use crate::native::{NativeCatalog, Store};

/// How long a runner's signed download URL stays valid.
const SIGNED_FOR: Duration = Duration::from_secs(300);

/// How many uploads run at once.
const UPLOADS: usize = 4;

/// The key prefix of every command package object in the store
/// (`storage.md` § The object store).
pub const PREFIX: &str = "native";

/// What a compressed copy's name adds to its executable's in a release
/// (`builds-and-releases.md` § Packaging).
const COMPRESSED_SUFFIX: &str = ".zst";

/// The metadata keys of the SHA-256 and the size of what an object stands
/// for.
const SHA256: &str = "sha256";
const SIZE: &str = "size";

/// What a package key leaves unencoded in a version: what a URI component
/// may carry as it is.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// One release directory of `commands/`: `descriptor.json`, and one
/// executable per target triple, named as the directory (with `.exe` for
/// Windows), beside its compressed copy.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Release {
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
    #[error("the object store failed: {0}")]
    Store(#[from] object_store::Error),
    #[error("native artifact publication was interrupted")]
    Cancelled,
}

/// Publishes the releases in `commands`, a server release's `commands/`,
/// into `objects`, and makes the catalog of them, which the conversations'
/// commands bind to. The backend accepts requests only once this completed;
/// `cancel` interrupts it.
pub async fn publish_native(
    commands: &FilePath,
    objects: &Objects,
    cancel: &CancellationToken,
) -> Result<NativeCatalog, PublicationError> {
    let releases = releases(commands).await?;
    let published = publish(&releases, &*objects.store, cancel).await?;
    let store = match &objects.signer {
        Some(signer) => Store::Signed(SignedArtifacts {
            signer: signer.clone(),
            published: Arc::new(published.artifacts),
        }),
        None => Store::Local(Arc::new(LocalArtifacts::new(
            objects.store.clone(),
            published.artifacts,
        ))),
    };
    tracing::info!(
        packages = published.packages.len(),
        "the command packages are published"
    );
    NativeCatalog::new(published.packages, store).map_err(|error| {
        PublicationError::Commands {
            directory: commands.to_owned(),
            reason: error.to_string(),
        }
    })
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

/// A verified release: its descriptor, and its executables by target with
/// their compressed copies.
struct Verified {
    descriptor: PackageDescriptor,
    executables: Vec<(PathBuf, PackageArtifact)>,
}

/// The published catalog: the releases' descriptors, and the size of each
/// artifact runners may download, by SHA-256.
struct Published {
    packages: Vec<PackageDescriptor>,
    artifacts: HashMap<String, u64>,
}

/// Publishes `releases` to `store`, verifying every release before any
/// upload. `cancel` stops it.
async fn publish(
    releases: &[Release],
    store: &dyn ObjectStore,
    cancel: &CancellationToken,
) -> Result<Published, PublicationError> {
    let verified = verify_all(releases, cancel).await?;
    // Each executable once, however many releases carry it, stored as its
    // compressed copy in the content coding.
    let mut uploads: HashMap<String, (PathBuf, PackageArtifact)> = HashMap::new();
    for release in &verified {
        for (path, artifact) in &release.executables {
            uploads
                .entry(artifact.sha256.clone())
                .or_insert_with(|| (compressed(path), artifact.clone()));
        }
    }
    let artifacts: HashMap<String, u64> = uploads
        .iter()
        .map(|(sha256, (_, artifact))| (sha256.clone(), artifact.size))
        .collect();
    futures_util::stream::iter(uploads.into_values().map(Ok))
        .try_for_each_concurrent(UPLOADS, |(path, artifact)| async move {
            let key = blob(&artifact.sha256)?;
            if in_place(store, &key, &artifact, cancel).await? {
                return Ok(());
            }
            let read = tokio::select! {
                () = cancel.cancelled() => return Err(PublicationError::Cancelled),
                read = tokio::fs::read(&path) => read,
            };
            let refused = |reason: String| PublicationError::Release {
                directory: path.clone(),
                reason,
            };
            let bytes = Bytes::from(read.map_err(|error| refused(error.to_string()))?);
            let expected = demi_shared_artifacts::Digest {
                size: artifact.size,
                sha256: artifact.sha256.clone(),
            };
            // What is stored is what was verified, whatever changed the file
            // since: what the compressed copy decodes to.
            let checked = bytes.clone();
            tokio::task::spawn_blocking(move || {
                demi_shared_artifacts::check_encoded_blocking(&checked, &expected)
            })
            .await
            .map_err(|error| refused(error.to_string()))?
            .map_err(|error| refused(error.to_string()))?;
            let coding = Some(demi_shared_artifacts::CONTENT_CODING);
            put_immutable(store, &key, bytes, &artifact, coding, cancel).await
        })
        .await?;
    for release in &verified {
        let descriptor = &release.descriptor;
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
        let version = utf8_percent_encode(&descriptor.version, COMPONENT);
        let claim = object(format!(
            "{PREFIX}/packages/{}/{version}.json",
            descriptor.id
        ))?;
        put_immutable(store, &claim, body, &artifact, None, cancel).await?;
    }
    Ok(Published {
        packages: verified
            .into_iter()
            .map(|release| release.descriptor)
            .collect(),
        artifacts,
    })
}

/// Where the compressed copy of the executable at `path` lies.
fn compressed(path: &FilePath) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(COMPRESSED_SUFFIX);
    PathBuf::from(name)
}

/// Verifies every release: its descriptor, and the executable of each
/// target it carries. Two releases of one package are refused.
async fn verify_all(
    releases: &[Release],
    cancel: &CancellationToken,
) -> Result<Vec<Verified>, PublicationError> {
    let mut verified = Vec::with_capacity(releases.len());
    let mut ids = HashSet::new();
    for release in releases {
        let release = verify(release, cancel).await?;
        if !ids.insert(release.descriptor.id.clone()) {
            return Err(PublicationError::Release {
                directory: PathBuf::from(&release.descriptor.id),
                reason: "the package is released twice".into(),
            });
        }
        verified.push(release);
    }
    Ok(verified)
}

/// Reads a release's descriptor and checks the executable of each target it
/// carries against it.
async fn verify(
    release: &Release,
    cancel: &CancellationToken,
) -> Result<Verified, PublicationError> {
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
    let mut executables = Vec::with_capacity(descriptor.targets.len());
    for (target, expected) in &descriptor.targets {
        let suffix = if target.contains("windows") {
            ".exe"
        } else {
            ""
        };
        let path = release
            .directory
            .join(target)
            .join(format!("{}{suffix}", release.executable));
        let found = demi_shared_artifacts::digest(&path, expected.size, cancel)
            .await
            .map_err(|error| match error {
                demi_shared_artifacts::Error::Cancelled => PublicationError::Cancelled,
                error => refused(format!("{target}: {error}")),
            })?;
        if found.size != expected.size || found.sha256 != expected.sha256 {
            return Err(refused(format!(
                "{target}: the executable does not match the descriptor"
            )));
        }
        executables.push((path, expected.clone()));
    }
    Ok(Verified {
        descriptor,
        executables,
    })
}

/// The object at `key`, which is taken as it is: a version's percent
/// signs are part of its key.
fn object(key: String) -> Result<Path, PublicationError> {
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
async fn put_immutable(
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
async fn in_place(
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

/// The published executables' downloads, signed on each request. `Send`,
/// so each shard makes its resolver of it.
#[derive(Clone)]
pub struct SignedArtifacts {
    signer: Arc<dyn Signer>,
    /// Each published artifact's size, by SHA-256.
    published: Arc<HashMap<String, u64>>,
}

impl SignedArtifacts {
    /// A signed download of `artifact`, when it is a published executable.
    async fn location(&self, artifact: &PackageArtifact) -> Result<ArtifactLocation, String> {
        if self.published.get(&artifact.sha256) != Some(&artifact.size) {
            return Err("the artifact is not in the published package catalog".into());
        }
        let key = blob(&artifact.sha256).map_err(|error| error.to_string())?;
        let url = self
            .signer
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

impl ArtifactResolver for SignedArtifacts {
    fn resolve(
        &self,
        artifact: &PackageArtifact,
        _target: &str,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        let artifacts = self.clone();
        let artifact = artifact.clone();
        Box::pin(async move {
            tokio::select! {
                () = cancel.cancelled() => Err("the work that asked no longer runs".into()),
                location = artifacts.location(&artifact) => location,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use demi_command_protocol::TARGETS;
    use object_store::ObjectStoreExt as _;
    use object_store::aws::AmazonS3Builder;
    use sha2::{Digest as _, Sha256};

    use demi_backend_blobs::fake_s3::FakeS3;
    use demi_backend_blobs::local::LocalObjects;
    use demi_command_package_browser_protocol::PACKAGE as BROWSER_PACKAGE;

    use super::*;

    /// A release of `targets` in `commands/commands`, whose executables are
    /// test bytes, not programs, each with its compressed copy beside it.
    fn release(commands: &FilePath, targets: &[&str]) -> Release {
        let directory = commands.join("commands");
        let mut artifacts = serde_json::Map::new();
        for target in targets {
            let bytes = format!("test-only artifact {target}");
            let suffix = if target.contains("windows") {
                ".exe"
            } else {
                ""
            };
            std::fs::create_dir_all(directory.join(target)).unwrap();
            let executable = directory.join(target).join(format!("commands{suffix}"));
            std::fs::write(&executable, &bytes).unwrap();
            let encoded = demi_shared_artifacts::encode_blocking(
                bytes.as_bytes(),
                demi_shared_artifacts::Effort::Fast,
            )
            .unwrap();
            std::fs::write(compressed(&executable), encoded).unwrap();
            let artifact = serde_json::json!({ "sha256": hex::encode(Sha256::digest(&bytes)), "size": bytes.len() });
            artifacts.insert((*target).to_owned(), artifact);
        }
        let descriptor = serde_json::json!({
            "id": "example.commands", "version": "1.0.0+build", "protocolVersion": 1,
            "operations": ["fixture"], "targets": artifacts,
        });
        std::fs::write(directory.join("descriptor.json"), descriptor.to_string()).unwrap();
        Release {
            directory,
            executable: "commands".into(),
        }
    }

    const CLAIM: &str = "native/packages/example.commands/1.0.0%2Bbuild.json";

    #[tokio::test]
    async fn the_executables_precede_the_immutable_descriptor_and_a_version_keeps_its_meaning() {
        let fake = FakeS3::start().await;
        let store = fake.client();
        let commands = tempfile::tempdir().unwrap();
        let releases = [release(commands.path(), TARGETS)];
        let cancel = CancellationToken::new();
        let published = publish(&releases, &store, &cancel).await.unwrap();
        assert_eq!(published.packages[0].id, "example.commands");
        let written = fake.written();
        assert_eq!(written.len(), 8, "{written:?}");
        assert!(
            written[..6]
                .iter()
                .all(|key| key.starts_with("native/blobs/")),
            "{written:?}"
        );
        assert!(written[6].starts_with("native/descriptors/"), "{written:?}");
        assert_eq!(written[7], CLAIM);
        // An executable is stored as its compressed copy and downloads as
        // the executable its descriptor names.
        let executable = releases[0].directory.join(TARGETS[0]).join("commands");
        let bytes = std::fs::read(&executable).unwrap();
        let digest = demi_shared_artifacts::Digest {
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&bytes)),
        };
        let key = format!("native/blobs/{}", digest.sha256);
        assert_eq!(
            fake.object(&key).unwrap(),
            std::fs::read(compressed(&executable)).unwrap()
        );
        let mut downloaded = Vec::new();
        let client = demi_shared_artifacts::client_allowing_http().unwrap();
        let url = format!("{}demi/{key}", fake.endpoint);
        demi_shared_artifacts::download(&client, &url, &digest, &mut downloaded, &cancel)
            .await
            .unwrap();
        assert_eq!(downloaded, bytes);
        // A second start finds its objects in place.
        publish(&releases, &store, &cancel).await.unwrap();
        assert_eq!(fake.written().len(), 8);

        // The same version naming other operations is refused, and the
        // version names what it named.
        let descriptor = releases[0].directory.join("descriptor.json");
        let changed = std::fs::read_to_string(&descriptor)
            .unwrap()
            .replace(r#"["fixture"]"#, r#"["fixture","changed"]"#);
        std::fs::write(&descriptor, changed).unwrap();
        let refused = publish(&releases, &store, &cancel).await;
        assert!(
            matches!(refused, Err(PublicationError::Conflict(_))),
            "{:?}",
            refused.err()
        );
        let claim: serde_json::Value =
            serde_json::from_slice(&fake.object(CLAIM).unwrap()).unwrap();
        assert_eq!(claim["operations"], serde_json::json!(["fixture"]));
    }

    #[tokio::test]
    async fn a_release_that_cannot_be_published_whole_publishes_no_descriptor() {
        let fake = FakeS3::start().await;
        let store = fake.client();
        let cancel = CancellationToken::new();
        let commands = tempfile::tempdir().unwrap();
        let releases = [release(commands.path(), TARGETS)];
        let windows = releases[0].directory.join(TARGETS[5]).join("commands.exe");
        let bytes = std::fs::read(&windows).unwrap();
        std::fs::write(&windows, "corrupt").unwrap();
        let corrupt = publish(&releases, &store, &cancel).await.err().unwrap();
        assert!(
            corrupt
                .to_string()
                .contains("does not match the descriptor"),
            "{corrupt}"
        );
        assert!(fake.written().is_empty(), "{:?}", fake.written());

        // A compressed copy that is not the executable's is refused, and the
        // version is not published.
        std::fs::write(&windows, &bytes).unwrap();
        std::fs::write(compressed(&windows), "not zstd").unwrap();
        let undecodable = publish(&releases, &store, &cancel).await.err().unwrap();
        assert!(
            matches!(undecodable, PublicationError::Release { .. }),
            "{undecodable}"
        );
        assert!(
            fake.written()
                .iter()
                .all(|key| key.starts_with("native/blobs/")),
            "{:?}",
            fake.written()
        );

        // An executable's object that holds other bytes stops the release
        // before its descriptor.
        let other = tempfile::tempdir().unwrap();
        let releases = [release(other.path(), TARGETS)];
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let squatted = Path::parse(format!("native/blobs/{sha256}")).unwrap();
        store
            .put(&squatted, PutPayload::from_static(b"other bytes"))
            .await
            .unwrap();
        let conflict = publish(&releases, &store, &cancel).await;
        assert!(
            matches!(conflict, Err(PublicationError::Conflict(_))),
            "{:?}",
            conflict.err()
        );
        assert!(
            fake.written()
                .iter()
                .all(|key| key.starts_with("native/blobs/")),
            "{:?}",
            fake.written()
        );
    }

    #[tokio::test]
    async fn a_published_executable_downloads_from_an_https_url_signed_for_five_minutes() {
        let s3 = AmazonS3Builder::new()
            .with_bucket_name("demi-native")
            .with_region("us-east-1")
            .with_access_key_id("fixture")
            .with_secret_access_key("fixture")
            .build()
            .unwrap();
        let artifact = PackageArtifact {
            sha256: "0".repeat(64),
            size: 10,
        };
        let artifacts = SignedArtifacts {
            signer: Arc::new(s3),
            published: Arc::new(HashMap::from([(artifact.sha256.clone(), artifact.size)])),
        };
        let asked = jiff::Timestamp::now();
        let ArtifactLocation::Url(location) = artifacts.location(&artifact).await.unwrap() else {
            panic!("a published executable downloads from a URL");
        };
        let url = url::Url::parse(&location.url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(
            url.path()
                .ends_with(&format!("/native/blobs/{}", artifact.sha256)),
            "{url}"
        );
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "X-Amz-Expires" && value == "300"),
            "{url}"
        );
        let valid_for = location.expires_at.unwrap() - asked.as_millisecond();
        assert!((300_000..301_000).contains(&valid_for), "{valid_for}");
        let unpublished = PackageArtifact {
            size: 11,
            ..artifact.clone()
        };
        let refused = artifacts.location(&unpublished).await.err().unwrap();
        assert!(refused.contains("not in the published"), "{refused}");
    }

    #[tokio::test]
    async fn a_local_store_holds_what_s3_would_and_the_backend_serves_it() {
        let data = tempfile::tempdir().unwrap();
        let objects = Objects {
            store: Arc::new(LocalObjects::new(data.path()).unwrap()),
            signer: None,
        };
        let commands = tempfile::tempdir().unwrap();
        let releases = [release(commands.path(), &TARGETS[..2])];
        let cancel = CancellationToken::new();
        let catalog = publish_native(commands.path(), &objects, &cancel)
            .await
            .unwrap();
        assert!(catalog.package("example.commands").is_some());
        let executable = releases[0].directory.join(TARGETS[0]).join("commands");
        let bytes = std::fs::read(&executable).unwrap();
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let stored = catalog.local_artifact(&sha256).await.unwrap().unwrap();
        assert_eq!(
            stored.attributes.get(&Attribute::ContentEncoding).map(|value| value.as_ref()),
            Some("zstd")
        );
        assert_eq!(
            stored.bytes().await.unwrap(),
            std::fs::read(compressed(&executable)).unwrap()
        );
        assert!(catalog.local_artifact(&"0".repeat(64)).await.is_none());
        // A second start finds its objects in place, as on S3.
        publish_native(commands.path(), &objects, &cancel)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_commands_directory_names_each_release_and_an_empty_one_serves_no_package() {
        let fake = FakeS3::start().await;
        let objects = Objects {
            store: Arc::new(fake.client()),
            signer: None,
        };
        let cancel = CancellationToken::new();
        let commands = tempfile::tempdir().unwrap();
        let catalog = publish_native(commands.path(), &objects, &cancel)
            .await
            .unwrap();
        assert!(catalog.package(BROWSER_PACKAGE).is_none());
        assert!(!catalog.serves(BROWSER_PACKAGE, &[]));
        let missing = publish_native(&commands.path().join("absent"), &objects, &cancel).await;
        assert!(
            matches!(missing, Err(PublicationError::Commands { .. })),
            "{:?}",
            missing.err()
        );
        std::fs::create_dir(commands.path().join("not a name")).unwrap();
        let refused = publish_native(commands.path(), &objects, &cancel).await;
        assert!(
            matches!(refused, Err(PublicationError::Commands { .. })),
            "{:?}",
            refused.err()
        );
    }
}
