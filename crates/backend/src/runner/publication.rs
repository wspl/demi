//! Native artifact publication (`native-runtime.md` § Publish artifacts
//! before enabling commands, § Backend deployment configuration): before the
//! backend accepts requests, every native command release `DEMI_NATIVE_CONFIG`
//! names is verified whole (its descriptor, and each of the six targets'
//! executable by size and SHA-256), its executables are uploaded once each as
//! content-addressed objects, its descriptor and then its immutable
//! package-and-version mapping are published, and only then does the
//! catalog serve. A write never replaces an object: finding one in place is
//! success when its size and digest are the ones published, and a refusal
//! otherwise. Runners download an executable from a URL signed for five
//! minutes.

use std::collections::{HashMap, HashSet};
use std::path::{Path as FilePath, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use bytes::Bytes;
use demi_command_service::protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact, PackageDescriptor, TARGETS};
use demi_host_remote::ArtifactResolver;
use futures_util::TryStreamExt as _;
use futures_util::future::LocalBoxFuture;
use object_store::path::Path;
use object_store::signer::Signer;
use object_store::{Attribute, Attributes, GetOptions, ObjectStore, PutMode, PutOptions, PutPayload};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::native::NativeCatalog;
use crate::storage::objects::S3Config;

/// How long a runner's signed download URL stays valid.
const SIGNED_FOR: Duration = Duration::from_secs(300);

/// How many uploads run at once.
const UPLOADS: usize = 4;

/// The metadata key an object's SHA-256 is published under.
const SHA256: &str = "sha256";

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

/// `DEMI_NATIVE_CONFIG`: the releases to publish and where.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeConfig {
    /// The key prefix of every published object.
    #[serde(default = "default_prefix")]
    prefix: String,
    releases: Vec<Release>,
    store: NativeStore,
}

fn default_prefix() -> String {
    "native".into()
}

/// One release directory: `descriptor.json`, and one executable per target
/// triple, named `executable` (with `.exe` for Windows).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {
    directory: PathBuf,
    executable: String,
}

/// The object storage the releases are published to; S3 is the one
/// protocol.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
enum NativeStore {
    S3(S3Config),
}

/// Why publication stopped the start.
#[derive(Debug, thiserror::Error)]
pub enum PublicationError {
    #[error("DEMI_NATIVE_CONFIG cannot be used: {0}")]
    Config(String),
    #[error("the native release in {} cannot be published: {reason}", directory.display())]
    Release { directory: PathBuf, reason: String },
    #[error("an immutable native artifact object differs from the one in place: {0}")]
    Conflict(String),
    #[error("the native artifact store failed: {0}")]
    Store(#[from] object_store::Error),
    #[error("native artifact publication was interrupted")]
    Cancelled,
}

/// Publishes the releases the configuration file at `path` names and makes
/// the catalog of them, which the conversations' commands bind to. The
/// backend accepts requests only once this completed; `cancel` interrupts
/// it.
pub async fn publish_native(path: &FilePath, cancel: &CancellationToken) -> Result<NativeCatalog, PublicationError> {
    let config = NativeConfig::read(path).await?;
    let NativeStore::S3(s3) = &config.store;
    let store = Arc::new(s3.builder().build()?);
    let published = publish(&config.releases, &config.prefix, store, cancel).await?;
    let artifacts = published.artifacts;
    let resolver = move || Rc::new(artifacts.clone()) as Rc<dyn ArtifactResolver>;
    NativeCatalog::new(published.packages, resolver).map_err(|error| PublicationError::Config(error.to_string()))
}

impl NativeConfig {
    /// The configuration in the JSON file at `path`; relative release
    /// directories resolve against the file's directory.
    async fn read(path: &FilePath) -> Result<Self, PublicationError> {
        let invalid = |reason: String| PublicationError::Config(format!("{}: {reason}", path.display()));
        let bytes = tokio::fs::read(path).await.map_err(|error| invalid(error.to_string()))?;
        let mut config: Self = serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
        // Segments of letters, digits, `_` and `-`, beginning with a letter
        // or digit.
        let prefix_ok = config
            .prefix
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && config.prefix.split('/').all(|segment| {
                !segment.is_empty()
                    && segment
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            });
        if !prefix_ok {
            return Err(invalid(format!("{} is no object key prefix", config.prefix)));
        }
        let base = path.parent().unwrap_or(FilePath::new(""));
        for release in &mut config.releases {
            let basename = !release.executable.is_empty()
                && release
                    .executable
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
            if !basename {
                return Err(invalid(format!("{} is no executable basename", release.executable)));
            }
            release.directory = base.join(&release.directory);
        }
        let NativeStore::S3(s3) = &config.store;
        s3.check().map_err(|error| invalid(error.to_string()))?;
        Ok(config)
    }
}

/// A verified release: its descriptor, and its executables by target.
struct Verified {
    descriptor: PackageDescriptor,
    executables: Vec<(PathBuf, PackageArtifact)>,
}

/// The published catalog: the releases' descriptors, and the executables
/// runners may download.
struct Published {
    packages: Vec<PackageDescriptor>,
    artifacts: SignedArtifacts,
}

/// Publishes `releases` to `store` under `prefix`, verifying every release
/// before any upload. `cancel` stops it.
async fn publish<S>(
    releases: &[Release],
    prefix: &str,
    store: Arc<S>,
    cancel: &CancellationToken,
) -> Result<Published, PublicationError>
where
    S: ObjectStore + Signer,
{
    let mut verified = Vec::with_capacity(releases.len());
    let mut ids = HashSet::new();
    for release in releases {
        let release = verify(release, cancel).await?;
        if !ids.insert(release.descriptor.id.clone()) {
            return Err(PublicationError::Config(format!("{} is released twice", release.descriptor.id)));
        }
        verified.push(release);
    }
    // Each executable once, however many releases carry it.
    let mut uploads: HashMap<String, (PathBuf, PackageArtifact)> = HashMap::new();
    for release in &verified {
        for (path, artifact) in &release.executables {
            uploads
                .entry(artifact.sha256.clone())
                .or_insert_with(|| (path.clone(), artifact.clone()));
        }
    }
    let published: HashMap<String, u64> = uploads
        .iter()
        .map(|(sha256, (_, artifact))| (sha256.clone(), artifact.size))
        .collect();
    futures_util::stream::iter(uploads.into_values().map(Ok))
        .try_for_each_concurrent(UPLOADS, |(path, artifact)| {
            let store = &*store;
            async move {
                let read = tokio::select! {
                    () = cancel.cancelled() => return Err(PublicationError::Cancelled),
                    read = tokio::fs::read(&path) => read,
                };
                let refused = |reason: String| PublicationError::Release {
                    directory: path.clone(),
                    reason,
                };
                let bytes = Bytes::from(read.map_err(|error| refused(error.to_string()))?);
                // What is uploaded is what was verified, whatever changed the
                // file since.
                let mut verifier = demi_artifact::Verifier::new(&demi_artifact::Digest {
                    size: artifact.size,
                    sha256: artifact.sha256.clone(),
                });
                verifier
                    .update(&bytes)
                    .and_then(|()| verifier.finish())
                    .map_err(|error| refused(error.to_string()))?;
                put_immutable(store, &blob(prefix, &artifact.sha256)?, bytes, &artifact, cancel).await
            }
        })
        .await?;
    for release in &verified {
        let descriptor = &release.descriptor;
        let body = serde_json_canonicalizer::to_vec(descriptor).map_err(|error| PublicationError::Config(error.to_string()))?;
        let digest = descriptor
            .digest()
            .map_err(|error| PublicationError::Config(error.to_string()))?;
        let artifact = PackageArtifact {
            sha256: digest,
            size: body.len() as u64,
        };
        let body = Bytes::from(body);
        let named = object(format!("{prefix}/descriptors/{}.json", artifact.sha256))?;
        put_immutable(&*store, &named, body.clone(), &artifact, cancel).await?;
        // The package and version's meaning, published last: a conflicting
        // descriptor fails instead of changing what the version names.
        let version = utf8_percent_encode(&descriptor.version, COMPONENT);
        let claim = object(format!("{prefix}/packages/{}/{version}.json", descriptor.id))?;
        put_immutable(&*store, &claim, body, &artifact, cancel).await?;
    }
    Ok(Published {
        packages: verified.into_iter().map(|release| release.descriptor).collect(),
        artifacts: SignedArtifacts {
            signer: store,
            prefix: prefix.to_owned(),
            published: Arc::new(published),
        },
    })
}

/// Reads a release's descriptor and checks every target's executable
/// against it.
async fn verify(release: &Release, cancel: &CancellationToken) -> Result<Verified, PublicationError> {
    let refused = |reason: String| PublicationError::Release {
        directory: release.directory.clone(),
        reason,
    };
    let text = tokio::fs::read(release.directory.join("descriptor.json"))
        .await
        .map_err(|error| refused(format!("descriptor.json: {error}")))?;
    let value = serde_json::from_slice(&text).map_err(|error| refused(format!("descriptor.json: {error}")))?;
    let descriptor = PackageDescriptor::parse(value).map_err(|error| refused(format!("descriptor.json: {error}")))?;
    let mut executables = Vec::with_capacity(TARGETS.len());
    for target in TARGETS {
        let expected = descriptor
            .targets
            .get(*target)
            .ok_or_else(|| refused(format!("it lacks a target: {target}")))?;
        let suffix = if target.contains("windows") { ".exe" } else { "" };
        let path = release
            .directory
            .join(target)
            .join(format!("{}{suffix}", release.executable));
        let found = demi_artifact::digest(&path, expected.size, cancel)
            .await
            .map_err(|error| match error {
                demi_artifact::Error::Cancelled => PublicationError::Cancelled,
                error => refused(format!("{target}: {error}")),
            })?;
        if found.size != expected.size || found.sha256 != expected.sha256 {
            return Err(refused(format!("{target}: the executable does not match the descriptor")));
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
    Path::parse(&key).map_err(|error| PublicationError::Config(error.to_string()))
}

/// A content-addressed executable's key.
fn blob(prefix: &str, sha256: &str) -> Result<Path, PublicationError> {
    object(format!("{prefix}/blobs/{sha256}"))
}

/// Creates the object at `path` unless one is there; one in place is
/// success when it is these bytes, by size and published digest, and a
/// conflict otherwise.
async fn put_immutable(
    store: &dyn ObjectStore,
    path: &Path,
    bytes: Bytes,
    artifact: &PackageArtifact,
    cancel: &CancellationToken,
) -> Result<(), PublicationError> {
    let mut attributes = Attributes::new();
    attributes.insert(Attribute::Metadata(SHA256.into()), artifact.sha256.clone().into());
    attributes.insert(Attribute::ContentType, "application/octet-stream".into());
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
            let head = GetOptions {
                head: true,
                ..GetOptions::default()
            };
            let existing = store.get_opts(path, head).await?;
            let digest = existing.attributes.get(&Attribute::Metadata(SHA256.into()));
            let same = existing.meta.size == artifact.size && digest.map(|value| value.as_ref()) == Some(artifact.sha256.as_str());
            if !same {
                return Err(PublicationError::Conflict(path.to_string()));
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// The published executables' downloads, signed on each request. `Send`,
/// so each shard makes its resolver of it.
#[derive(Clone)]
struct SignedArtifacts {
    signer: Arc<dyn Signer>,
    prefix: String,
    /// Each published executable's size, by SHA-256.
    published: Arc<HashMap<String, u64>>,
}

impl SignedArtifacts {
    /// A signed download of `artifact`, when it is a published executable.
    async fn location(&self, artifact: &PackageArtifact) -> Result<ArtifactLocation, String> {
        if self.published.get(&artifact.sha256) != Some(&artifact.size) {
            return Err("the artifact is not in the published package catalog".into());
        }
        let key = blob(&self.prefix, &artifact.sha256).map_err(|error| error.to_string())?;
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
    use object_store::ObjectStoreExt as _;
    use object_store::aws::AmazonS3Builder;
    use sha2::{Digest as _, Sha256};

    use demi_coding_agent::BUILTIN_PACKAGE;

    use super::*;
    use crate::storage::objects::fake_s3::FakeS3;

    /// A release of `targets` in `directory`, whose executables are test
    /// bytes, not programs.
    fn release(directory: &FilePath, targets: &[&str]) -> Release {
        let mut artifacts = serde_json::Map::new();
        for target in targets {
            let bytes = format!("test-only artifact {target}");
            let suffix = if target.contains("windows") { ".exe" } else { "" };
            std::fs::create_dir_all(directory.join(target)).unwrap();
            std::fs::write(directory.join(target).join(format!("commands{suffix}")), &bytes).unwrap();
            let artifact = serde_json::json!({ "sha256": hex::encode(Sha256::digest(&bytes)), "size": bytes.len() });
            artifacts.insert((*target).to_owned(), artifact);
        }
        let descriptor = serde_json::json!({
            "id": "example.commands", "version": "1.0.0+build", "protocolVersion": 1,
            "operations": ["fixture"], "targets": artifacts,
        });
        std::fs::write(directory.join("descriptor.json"), descriptor.to_string()).unwrap();
        Release {
            directory: directory.to_owned(),
            executable: "commands".into(),
        }
    }

    const CLAIM: &str = "native/packages/example.commands/1.0.0%2Bbuild.json";

    #[tokio::test]
    async fn the_six_executables_precede_the_immutable_descriptor_and_a_version_keeps_its_meaning() {
        let fake = FakeS3::start().await;
        let store = Arc::new(fake.client());
        let directory = tempfile::tempdir().unwrap();
        let releases = [release(directory.path(), TARGETS)];
        let cancel = CancellationToken::new();
        let published = publish(&releases, "native", store.clone(), &cancel).await.unwrap();
        assert_eq!(published.packages[0].id, "example.commands");
        let written = fake.written();
        assert_eq!(written.len(), 8, "{written:?}");
        assert!(written[..6].iter().all(|key| key.starts_with("native/blobs/")), "{written:?}");
        assert!(written[6].starts_with("native/descriptors/"), "{written:?}");
        assert_eq!(written[7], CLAIM);
        // A second start finds its objects in place.
        publish(&releases, "native", store.clone(), &cancel).await.unwrap();
        assert_eq!(fake.written().len(), 8);

        // The same version naming other operations is refused, and the
        // version names what it named.
        let changed = std::fs::read_to_string(directory.path().join("descriptor.json"))
            .unwrap()
            .replace(r#"["fixture"]"#, r#"["fixture","changed"]"#);
        std::fs::write(directory.path().join("descriptor.json"), changed).unwrap();
        let refused = publish(&releases, "native", store, &cancel).await;
        assert!(matches!(refused, Err(PublicationError::Conflict(_))), "{:?}", refused.err());
        let claim: serde_json::Value = serde_json::from_slice(&fake.object(CLAIM).unwrap()).unwrap();
        assert_eq!(claim["operations"], serde_json::json!(["fixture"]));
    }

    #[tokio::test]
    async fn a_release_that_cannot_be_published_whole_publishes_nothing() {
        let fake = FakeS3::start().await;
        let store = Arc::new(fake.client());
        let cancel = CancellationToken::new();
        let directory = tempfile::tempdir().unwrap();
        let releases = [release(directory.path(), TARGETS)];
        let windows = directory.path().join(TARGETS[5]).join("commands.exe");
        let bytes = std::fs::read(&windows).unwrap();
        std::fs::write(&windows, "corrupt").unwrap();
        let corrupt = publish(&releases, "native", store.clone(), &cancel).await.err().unwrap();
        assert!(corrupt.to_string().contains("does not match the descriptor"), "{corrupt}");
        std::fs::remove_file(&windows).unwrap();
        assert!(publish(&releases, "native", store.clone(), &cancel).await.is_err());
        assert!(fake.written().is_empty(), "{:?}", fake.written());

        // A development release of fewer targets.
        let development = tempfile::tempdir().unwrap();
        let partial = [release(development.path(), &TARGETS[..2])];
        let refused = publish(&partial, "native", store.clone(), &cancel).await.err().unwrap();
        assert!(refused.to_string().contains(&format!("it lacks a target: {}", TARGETS[2])), "{refused}");
        assert!(fake.written().is_empty(), "{:?}", fake.written());

        // An executable's object that holds other bytes stops the release
        // before its descriptor.
        std::fs::write(&windows, &bytes).unwrap();
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let squatted = Path::parse(format!("native/blobs/{sha256}")).unwrap();
        store.put(&squatted, PutPayload::from_static(b"other bytes")).await.unwrap();
        let conflict = publish(&releases, "native", store, &cancel).await;
        assert!(matches!(conflict, Err(PublicationError::Conflict(_))), "{:?}", conflict.err());
        assert!(fake.written().iter().all(|key| key.starts_with("native/blobs/")), "{:?}", fake.written());
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
            prefix: "native".into(),
            published: Arc::new(HashMap::from([(artifact.sha256.clone(), artifact.size)])),
        };
        let asked = jiff::Timestamp::now();
        let ArtifactLocation::Url(location) = artifacts.location(&artifact).await.unwrap() else {
            panic!("a published executable downloads from a URL");
        };
        let url = url::Url::parse(&location.url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(url.path().ends_with(&format!("/native/blobs/{}", artifact.sha256)), "{url}");
        assert!(url.query_pairs().any(|(key, value)| key == "X-Amz-Expires" && value == "300"), "{url}");
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
    async fn the_configuration_names_releases_beside_it_and_an_s3_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.json");
        let write = |text: &str| std::fs::write(&path, text).unwrap();
        write(r#"{"releases":[{"directory":"./demi-commands","executable":"demi-commands"}],"store":{"provider":"s3","bucket":"demi-native","region":"us-east-1"}}"#);
        let config = NativeConfig::read(&path).await.unwrap();
        assert_eq!(config.prefix, "native");
        assert_eq!(config.releases[0].directory, directory.path().join("./demi-commands"));
        for refused in [
            r#"{"store":{"provider":"s3","bucket":"b","region":"r"}}"#,
            r#"{"releases":[{"directory":"d","executable":"a.exe"}],"store":{"provider":"s3","bucket":"b","region":"r"}}"#,
            r#"{"releases":[{"directory":"d","executable":"a"}],"store":{"provider":"gcs","bucket":"b","region":"r"}}"#,
            r#"{"prefix":"native/","releases":[{"directory":"d","executable":"a"}],"store":{"provider":"s3","bucket":"b","region":"r"}}"#,
            r#"{"prefix":"a//b","releases":[{"directory":"d","executable":"a"}],"store":{"provider":"s3","bucket":"b","region":"r"}}"#,
            r#"{"releases":[{"directory":"d","executable":"a"}],"store":{"provider":"s3","bucket":"b","region":"r","endpoint":"http://s3.test"}}"#,
        ] {
            write(refused);
            assert!(NativeConfig::read(&path).await.is_err(), "{refused}");
        }
    }

    #[tokio::test]
    async fn an_explicit_empty_release_list_publishes_nothing_and_serves_no_package() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.json");
        std::fs::write(&path, r#"{"releases":[],"store":{"provider":"s3","bucket":"demi-native","region":"us-east-1"}}"#).unwrap();
        let catalog = publish_native(&path, &CancellationToken::new()).await.unwrap();
        assert!(catalog.package(BUILTIN_PACKAGE).is_none());
        assert!(!catalog.serves(BUILTIN_PACKAGE, &[]));
    }
}
