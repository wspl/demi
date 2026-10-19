//! Native package publication (`native-runtime.md` § Publish packages, then
//! source artifacts on demand, § Backend deployment configuration): before
//! the backend accepts requests, every command package release in its server
//! release's `commands/` is read and checked, its descriptor and then its
//! immutable package-and-version mapping are published into the
//! deployment's one object store, local or S3 alike, and only then does the
//! catalog serve. No executable is stored here: each enters the store the
//! first time something needs it ([`crate::sourcing`]). Every object carries
//! the SHA-256 and size of what it stands for, the executable's for an
//! encoded one. A write never replaces an object: finding one in place is
//! success when those are the ones published, and a refusal otherwise.

use std::collections::{HashMap, HashSet};
use std::path::{Path as FilePath, PathBuf};

use bytes::Bytes;
use demi_backend_blobs::store::Objects;
use demi_command_protocol::{PackageArtifact, PackageDescriptor};
use demi_runner_protocol::release::{
    FilesLocation, RunnerRelease, SERVER_RELEASE, ServerRelease, compressed_file,
};
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
/// (`storage.md` § The object store).
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
    #[error("the server release's files cannot be reached: {0}")]
    Files(String),
    #[error("the server release's record {} cannot be read: {reason}", path.display())]
    Record { path: PathBuf, reason: String },
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

/// Where the files of the server release whose root is `release` are, as
/// its `release.json` says.
pub async fn release_files(release: &FilePath) -> Result<FilesLocation, PublicationError> {
    let path = release.join(SERVER_RELEASE);
    let unreadable = |reason: String| PublicationError::Record {
        path: path.clone(),
        reason,
    };
    let record = tokio::fs::read(&path)
        .await
        .map_err(|error| unreadable(error.to_string()))?;
    let record = ServerRelease::decode(&record).map_err(|error| unreadable(error.to_string()))?;
    Ok(record.location())
}

/// Publishes the releases in `commands`, a server release's `commands/`,
/// into `objects`, and makes the catalog of them, which the conversations'
/// commands bind to and whose executables come from `files`, the release's
/// files. Then it records server release `version`, whose runner release is
/// `runners`, and removes the packages of older releases
/// ([`crate::retention`]). The backend accepts requests only once this
/// completed; `cancel` interrupts it.
pub async fn publish_native(
    commands: &FilePath,
    files: FilesLocation,
    version: &str,
    runners: Option<&RunnerRelease>,
    objects: &Objects,
    cancel: &CancellationToken,
) -> Result<NativeCatalog, PublicationError> {
    let releases = releases(commands).await?;
    let verified = verify_all(&releases).await?;
    publish(&verified, &*objects.store, cancel).await?;
    match keep_releases(&*objects.store, version, &verified, runners, cancel).await {
        Ok(pruned) => tracing::info!(
            records = pruned.records,
            mappings = pruned.mappings,
            descriptors = pruned.descriptors,
            blobs = pruned.blobs,
            "the native store keeps this release and the one before"
        ),
        Err(PublicationError::Cancelled) => return Err(PublicationError::Cancelled),
        // What stays is only stored, never served wrongly; the next start
        // prunes again.
        Err(error) => tracing::warn!("the native store's older releases stay: {error}"),
    }
    let mut executables = HashMap::new();
    for (release, descriptor) in releases.iter().zip(&verified) {
        for (target, artifact) in &descriptor.targets {
            let wanted = ReleaseArtifact {
                file: compressed_file(&release.executable, target),
                artifact: artifact.clone(),
                encoded: true,
            };
            executables.entry(artifact.sha256.clone()).or_insert(wanted);
        }
    }
    let sourcing = Sourcing::new(objects.store.clone(), files)?;
    let artifacts = Artifacts::new(sourcing, objects.signer.clone(), executables);
    tracing::info!(
        packages = verified.len(),
        "the command packages are published"
    );
    NativeCatalog::new(verified, artifacts).map_err(|error| PublicationError::Commands {
        directory: commands.to_owned(),
        reason: error.to_string(),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use demi_command_protocol::{ArtifactLocation, TARGETS};
    use object_store::ObjectStoreExt as _;
    use object_store::aws::AmazonS3Builder;
    use sha2::{Digest as _, Sha256};
    use tempfile::TempDir;

    use demi_backend_blobs::fake_s3::FakeS3;
    use demi_backend_blobs::local::LocalObjects;
    use demi_command_package_browser_protocol::PACKAGE as BROWSER_PACKAGE;

    use crate::public_url::PublicUrl;

    use super::*;

    /// A server release's `commands/`, with one release named `commands`
    /// of `targets`, and the release's files: each target's executable,
    /// test bytes rather than a program, as its compressed copy.
    struct Fixture {
        commands: TempDir,
        files: TempDir,
    }

    impl Fixture {
        fn new(targets: &[&str]) -> Self {
            Self::release("1.0.0+build", targets, executable)
        }

        /// A release whose package is of `version` and whose executable for
        /// a target is `executable`'s bytes.
        fn release(version: &str, targets: &[&str], executable: impl Fn(&str) -> Vec<u8>) -> Self {
            let commands = tempfile::tempdir().unwrap();
            let files = tempfile::tempdir().unwrap();
            let release = commands.path().join("commands");
            std::fs::create_dir_all(&release).unwrap();
            let mut artifacts = serde_json::Map::new();
            for target in targets {
                let bytes = executable(target);
                let encoded = demi_shared_artifacts::encode_blocking(
                    &bytes,
                    demi_shared_artifacts::Effort::Fast,
                )
                .unwrap();
                std::fs::write(files.path().join(compressed_file("commands", target)), encoded)
                    .unwrap();
                artifacts.insert((*target).to_owned(), serde_json::to_value(artifact_of(&bytes)).unwrap());
            }
            let descriptor = serde_json::json!({
                "id": "example.commands", "version": version, "protocolVersion": 1,
                "operations": ["fixture"], "targets": artifacts,
            });
            std::fs::write(release.join("descriptor.json"), descriptor.to_string()).unwrap();
            Self { commands, files }
        }

        fn files(&self) -> FilesLocation {
            FilesLocation::Directory(self.files.path().to_owned())
        }

        async fn publish(&self, objects: &Objects) -> Result<NativeCatalog, PublicationError> {
            self.publish_as(objects, "0.1.0", None).await
        }

        /// Publishes as server release `version`, whose runner release is
        /// `runners`, starts.
        async fn publish_as(
            &self,
            objects: &Objects,
            version: &str,
            runners: Option<&RunnerRelease>,
        ) -> Result<NativeCatalog, PublicationError> {
            let cancel = CancellationToken::new();
            publish_native(self.commands.path(), self.files(), version, runners, objects, &cancel)
                .await
        }
    }

    /// The test bytes standing for `target`'s executable.
    fn executable(target: &str) -> Vec<u8> {
        format!("test-only artifact {target}").into_bytes()
    }

    fn artifact(target: &str) -> PackageArtifact {
        artifact_of(&executable(target))
    }

    fn artifact_of(bytes: &[u8]) -> PackageArtifact {
        PackageArtifact {
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
        }
    }

    fn local(data: &FilePath) -> Objects {
        Objects {
            store: Arc::new(LocalObjects::new(data).unwrap()),
            signer: None,
        }
    }

    /// The backend's own URL, as once it listens.
    fn backend() -> PublicUrl {
        let backend = PublicUrl::default();
        backend.listening(None, "127.0.0.1:3271".parse().unwrap());
        backend
    }

    const CLAIM: &str = "native/packages/example.commands/1.0.0%2Bbuild.json";

    #[tokio::test]
    async fn a_start_publishes_the_descriptor_alone_and_a_version_keeps_its_meaning() {
        let fake = FakeS3::start().await;
        let objects = Objects {
            store: Arc::new(fake.client()),
            signer: None,
        };
        let fixture = Fixture::new(TARGETS);
        let catalog = fixture.publish(&objects).await.unwrap();
        assert!(catalog.package("example.commands").is_some());
        // Publishing stores no executable; the release's record, which each
        // start writes again, is not a package's.
        let packages = || -> Vec<String> {
            fake.written()
                .into_iter()
                .filter(|key| !key.starts_with("native/releases/"))
                .collect()
        };
        let written = packages();
        assert_eq!(written.len(), 2, "{written:?}");
        assert!(written[0].starts_with("native/descriptors/"), "{written:?}");
        assert_eq!(written[1], CLAIM);
        // A second start finds its objects in place.
        fixture.publish(&objects).await.unwrap();
        assert_eq!(packages().len(), 2);

        // The same version naming other operations is refused, and the
        // version names what it named.
        let descriptor = fixture.commands.path().join("commands/descriptor.json");
        let changed = std::fs::read_to_string(&descriptor)
            .unwrap()
            .replace(r#"["fixture"]"#, r#"["fixture","changed"]"#);
        std::fs::write(&descriptor, changed).unwrap();
        let refused = fixture.publish(&objects).await;
        assert!(
            matches!(refused, Err(PublicationError::Conflict(_))),
            "{:?}",
            refused.err()
        );
        let claim: serde_json::Value =
            serde_json::from_slice(&fake.object(CLAIM).unwrap()).unwrap();
        assert_eq!(claim["operations"], serde_json::json!(["fixture"]));
    }

    /// One server release of the pruning tests: its version, its package's
    /// release, its runner release, the executables a runner's need and an
    /// update store, and the keys of its record, mapping, descriptor and
    /// the two executables, in that order.
    struct Started {
        fixture: Fixture,
        version: String,
        runners: RunnerRelease,
        executables: [Vec<u8>; 2],
        keys: Vec<Path>,
    }

    impl Started {
        /// Server release `0.<index>.0`, whose package is `1.<index>.0`.
        fn new(index: u32) -> Self {
            let package = format!("1.{index}.0");
            let bytes = format!("artifact of {package}").into_bytes();
            let fixture = Fixture::release(&package, &TARGETS[..1], |_| bytes.clone());
            let runner = format!("runner of {package}").into_bytes();
            let runners = RunnerRelease {
                release: "a".repeat(64),
                wire: demi_runner_protocol::wire::VERSION,
                command_protocol: demi_command_protocol::VERSION,
                targets: [(TARGETS[0].to_owned(), artifact_of(&runner))].into(),
            };
            let version = format!("0.{index}.0");
            let text = std::fs::read(fixture.commands.path().join("commands/descriptor.json")).unwrap();
            let digest = PackageDescriptor::parse(serde_json::from_slice(&text).unwrap())
                .unwrap()
                .digest()
                .unwrap();
            let keys = vec![
                object(format!("native/releases/{version}.json")).unwrap(),
                mapping("example.commands", &package).unwrap(),
                object(format!("native/descriptors/{digest}.json")).unwrap(),
                blob(&artifact_of(&bytes).sha256).unwrap(),
                blob(&artifact_of(&runner).sha256).unwrap(),
            ];
            Self {
                fixture,
                version,
                runners,
                executables: [bytes, runner],
                keys,
            }
        }

        /// The release's start, then what a runner's need and an update
        /// store.
        async fn start(&self, objects: &Objects) {
            self.fixture
                .publish_as(objects, &self.version, Some(&self.runners))
                .await
                .unwrap();
            for stored in &self.executables {
                let key = blob(&artifact_of(stored).sha256).unwrap();
                objects.store.put(&key, stored.clone().into()).await.unwrap();
            }
        }

        /// Which of its keys the store holds.
        async fn held(&self, store: &dyn ObjectStore) -> Vec<bool> {
            let mut held = Vec::new();
            for key in &self.keys {
                held.push(store.head(key).await.is_ok());
            }
            held
        }
    }

    /// Both stores the pruning works on, a local one and the S3 fake.
    fn both_stores(data: &FilePath, fake: &FakeS3) -> [(&'static str, Objects); 2] {
        [
            ("local", local(data)),
            ("S3", Objects { store: Arc::new(fake.client()), signer: None }),
        ]
    }

    /// Three releases start one after another, each with its package's
    /// executable and its runner's stored, and a blob an interrupted
    /// publication left beside them. The third's start removes the first's
    /// record, mapping, descriptor and executables and the stray blob, and
    /// keeps its own and the second's. A rollback to the second then keeps
    /// the third, the release that ran before it. Local store and S3 alike.
    #[tokio::test]
    async fn a_start_keeps_its_release_and_the_one_before_and_removes_older_packages() {
        let data = tempfile::tempdir().unwrap();
        let fake = FakeS3::start().await;
        for (name, objects) in &both_stores(data.path(), &fake) {
            let store = &*objects.store;
            let releases: Vec<_> = (1..=3).map(Started::new).collect();
            releases[0].start(objects).await;
            releases[1].start(objects).await;
            let stray = blob(&"0".repeat(64)).unwrap();
            store.put(&stray, Bytes::from_static(b"interrupted").into()).await.unwrap();
            releases[2].start(objects).await;
            assert_eq!(releases[0].held(store).await, [false; 5], "{name}: the first release stays");
            assert_eq!(releases[1].held(store).await, [true; 5], "{name}: the release before goes");
            assert_eq!(releases[2].held(store).await, [true; 5], "{name}: the release goes");
            assert!(store.head(&stray).await.is_err(), "{name}: the stray blob stays");

            releases[1].start(objects).await;
            assert_eq!(releases[2].held(store).await, [true; 5], "{name}: the rolled-back release goes");
        }
    }

    /// The store lost the descriptor of the release before, as damage would.
    /// The next start cannot tell that release's executables from what no
    /// release names, so it removes nothing: not those executables, not the
    /// older release, not a stray blob.
    #[tokio::test]
    async fn a_start_on_a_store_that_lacks_a_kept_descriptor_removes_nothing() {
        let data = tempfile::tempdir().unwrap();
        let fake = FakeS3::start().await;
        for (name, objects) in &both_stores(data.path(), &fake) {
            let store = &*objects.store;
            let releases: Vec<_> = (1..=3).map(Started::new).collect();
            releases[0].start(objects).await;
            releases[1].start(objects).await;
            store.delete(&releases[1].keys[2]).await.unwrap();
            let stray = blob(&"0".repeat(64)).unwrap();
            store.put(&stray, Bytes::from_static(b"interrupted").into()).await.unwrap();
            releases[2].start(objects).await;
            assert_eq!(releases[0].held(store).await, [true; 5], "{name}: the older release went");
            assert_eq!(
                releases[1].held(store).await,
                [true, true, false, true, true],
                "{name}: the release before lost more"
            );
            assert!(store.head(&stray).await.is_ok(), "{name}: the stray blob went");
        }
    }

    /// The first `demi file edit` on a laptop of a target: the runner asks
    /// where the executable downloads from, and the backend takes it from
    /// the release's files, checks it, stores it and answers.
    #[tokio::test]
    async fn an_executable_enters_the_store_the_first_time_a_runner_needs_it() {
        let data = tempfile::tempdir().unwrap();
        let objects = local(data.path());
        let fixture = Fixture::new(&TARGETS[..2]);
        let catalog = fixture.publish(&objects).await.unwrap();
        let wanted = artifact(TARGETS[0]);
        // Nothing serves it before a runner needs it.
        assert!(catalog.local_artifact(&wanted.sha256, 0).await.is_none());
        let resolver = catalog.resolver(&backend());
        let cancel = CancellationToken::new();
        let location = resolver
            .resolve(&wanted, TARGETS[0], cancel.clone())
            .await
            .unwrap();
        let ArtifactLocation::Url(location) = location else {
            panic!("a runner downloads from a URL");
        };
        assert_eq!(
            location.url,
            format!("http://127.0.0.1:3271/native-artifacts/{}", wanted.sha256)
        );
        // The store holds the compressed copy, served in its content coding.
        let file = fixture.files.path().join(compressed_file("commands", TARGETS[0]));
        let Ok(crate::native::StoredBytes::Stored(stored)) =
            catalog.local_artifact(&wanted.sha256, 0).await.unwrap()
        else {
            panic!("the whole copy is stored");
        };
        assert_eq!(
            stored.attributes.get(&Attribute::ContentEncoding).map(|value| value.as_ref()),
            Some("zstd")
        );
        assert_eq!(stored.bytes().await.unwrap(), std::fs::read(&file).unwrap());
        // A later need finds it stored, without the release's files.
        std::fs::remove_file(&file).unwrap();
        resolver.resolve(&wanted, TARGETS[0], cancel.clone()).await.unwrap();

        // A copy that does not decode to the executable is not stored, and
        // the need fails with the reason.
        let other = artifact(TARGETS[1]);
        let file = fixture.files.path().join(compressed_file("commands", TARGETS[1]));
        std::fs::write(&file, "not zstd").unwrap();
        let refused = resolver.resolve(&other, TARGETS[1], cancel.clone()).await.err().unwrap();
        assert!(refused.contains("commands-"), "{refused}");
        assert!(catalog.local_artifact(&other.sha256, 0).await.is_none());
        // Nor is an artifact the catalog does not name.
        let unnamed = PackageArtifact {
            size: wanted.size + 1,
            ..wanted.clone()
        };
        let refused = resolver.resolve(&unnamed, TARGETS[0], cancel).await.err().unwrap();
        assert!(refused.contains("not in the published"), "{refused}");
    }

    /// After an upgrade, the backend stores the release's executables for
    /// the targets of the Hosts it knows before a command needs them, and a
    /// device of another target has its own stored as it pairs. A runner
    /// that asks for an executable while it is being stored waits for that
    /// one fetch.
    #[tokio::test]
    async fn executables_are_stored_ahead_for_the_hosts_targets_and_a_need_shares_the_fetch() {
        let fake = FakeS3::start().await;
        let objects = Objects {
            store: Arc::new(fake.client()),
            signer: None,
        };
        let fixture = Fixture::new(&TARGETS[..2]);
        let catalog = fixture.publish(&objects).await.unwrap();
        let mut writes = fake.hold_writes();
        let cancel = CancellationToken::new();
        // The backend starts knowing a Host of the first target.
        catalog.foresee([TARGETS[0]]);
        let ahead = {
            let catalog = catalog.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move { catalog.store_ahead(&cancel).await })
        };
        let first = artifact(TARGETS[0]);
        let key = |artifact: &PackageArtifact| blob(&artifact.sha256).unwrap().to_string();
        assert_eq!(writes.arrived().await, key(&first));

        // The first command's runner asks while the store is being written,
        // and waits for its turn.
        let resolver = catalog.resolver(&backend());
        let mut need = resolver.resolve(&first, TARGETS[0], CancellationToken::new());
        assert!(futures_util::poll!(&mut need).is_pending());
        writes.release();
        let ArtifactLocation::Url(location) = need.await.unwrap() else {
            panic!("a runner downloads from a URL");
        };
        assert!(location.url.ends_with(&first.sha256), "{}", location.url);
        assert_eq!(writes.more(), Vec::<String>::new(), "the need fetched again");
        // No Host of the second target is known, so nothing stored it.
        let second = artifact(TARGETS[1]);
        assert!(fake.object(&key(&second)).is_none());

        // A device of the second target pairs.
        catalog.foresee([TARGETS[1]]);
        assert_eq!(writes.arrived().await, key(&second));
        cancel.cancel();
        ahead.await.unwrap();
    }

    #[tokio::test]
    async fn an_s3_store_s_executable_downloads_from_an_https_url_signed_for_five_minutes() {
        let data = tempfile::tempdir().unwrap();
        let signer = AmazonS3Builder::new()
            .with_bucket_name("demi-native")
            .with_region("us-east-1")
            .with_access_key_id("fixture")
            .with_secret_access_key("fixture")
            .build()
            .unwrap();
        let objects = Objects {
            store: Arc::new(LocalObjects::new(data.path()).unwrap()),
            signer: Some(Arc::new(signer)),
        };
        let fixture = Fixture::new(&TARGETS[..1]);
        let catalog = fixture.publish(&objects).await.unwrap();
        let wanted = artifact(TARGETS[0]);
        let asked = jiff::Timestamp::now();
        let location = catalog
            .resolver(&backend())
            .resolve(&wanted, TARGETS[0], CancellationToken::new())
            .await
            .unwrap();
        let ArtifactLocation::Url(location) = location else {
            panic!("a runner downloads from a URL");
        };
        let url = url::Url::parse(&location.url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(
            url.path()
                .ends_with(&format!("/native/blobs/{}", wanted.sha256)),
            "{url}"
        );
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "X-Amz-Expires" && value == "300"),
            "{url}"
        );
        let valid_for = location.expires_at.unwrap() - asked.as_millisecond();
        assert!((300_000..301_000).contains(&valid_for), "{valid_for}");
        // The backend serves none of an S3 store's objects itself.
        assert!(catalog.local_artifact(&wanted.sha256, 0).await.is_none());
    }

    /// A runner executable's download: the runner release names it, and the
    /// backend takes its compressed copy from the release's files, checks
    /// what it decodes to, and keeps it in the content coding.
    #[tokio::test]
    async fn a_runner_executable_is_sourced_as_its_compressed_copy() {
        let data = tempfile::tempdir().unwrap();
        let objects = local(data.path());
        let fixture = Fixture::new(&TARGETS[..1]);
        let catalog = fixture.publish(&objects).await.unwrap();
        let target = TARGETS[5];
        let bytes = b"a runner".to_vec();
        let runner = PackageArtifact {
            sha256: hex::encode(Sha256::digest(&bytes)),
            size: bytes.len() as u64,
        };
        let file = fixture
            .files
            .path()
            .join(demi_runner_protocol::release::compressed_file("demi-runner", target));
        let encode = |bytes: &[u8]| {
            demi_shared_artifacts::encode_blocking(bytes, demi_shared_artifacts::Effort::Fast)
                .unwrap()
        };
        std::fs::write(&file, encode(b"another runner")).unwrap();
        let cancel = CancellationToken::new();
        let refused = catalog
            .runner_executable(target, &runner, None, 0, &cancel)
            .await
            .err()
            .unwrap();
        assert!(refused.contains("demi-runner-"), "{refused}");
        let encoded = encode(&bytes);
        std::fs::write(&file, &encoded).unwrap();
        let crate::native::StoredBytes::Stored(stored) = catalog
            .runner_executable(target, &runner, None, 0, &cancel)
            .await
            .unwrap()
        else {
            panic!("the whole copy is stored");
        };
        assert_eq!(
            stored.attributes.get(&Attribute::ContentEncoding).map(|coding| coding.as_ref()),
            Some(demi_shared_artifacts::CONTENT_CODING)
        );
        assert_eq!(stored.bytes().await.unwrap(), encoded);
    }

    #[tokio::test]
    async fn the_commands_directory_names_each_release_and_an_empty_one_serves_no_package() {
        let data = tempfile::tempdir().unwrap();
        let objects = local(data.path());
        let cancel = CancellationToken::new();
        let commands = tempfile::tempdir().unwrap();
        let files = || FilesLocation::Directory(commands.path().to_owned());
        let catalog = publish_native(commands.path(), files(), "0.1.0", None, &objects, &cancel)
            .await
            .unwrap();
        assert!(catalog.package(BROWSER_PACKAGE).is_none());
        assert!(!catalog.serves(BROWSER_PACKAGE, &[]));
        let absent = commands.path().join("absent");
        let missing = publish_native(&absent, files(), "0.1.0", None, &objects, &cancel).await;
        assert!(
            matches!(missing, Err(PublicationError::Commands { .. })),
            "{:?}",
            missing.err()
        );
        std::fs::create_dir(commands.path().join("not a name")).unwrap();
        let refused = publish_native(commands.path(), files(), "0.1.0", None, &objects, &cancel).await;
        assert!(
            matches!(refused, Err(PublicationError::Commands { .. })),
            "{:?}",
            refused.err()
        );
    }
}
