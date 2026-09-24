//! The object store blobs and the change store share (`storage.md` § The
//! object store): the data directory of a single-backend deployment, or the
//! S3 bucket `DEMI_CHANGE_STORE_CONFIG` names, reached through
//! `object_store` either way, so a blob is the object `blobs/<user>/<sha256>`
//! and the change store's objects are under `changes/`. S3's credentials
//! come from the standard AWS environment variables, a web identity token,
//! or container or instance metadata; no profile file is read.

use std::path::Path;
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::aws::{AmazonS3Builder, Checksum};
use object_store::local::LocalFileSystem;
use serde::Deserialize;
use url::Url;

use super::StorageError;

/// Where `DEMI_CHANGE_STORE_CONFIG` puts the object store: an S3 bucket.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct S3Config {
    pub(crate) bucket: String,
    pub(crate) region: String,
    /// An S3-compatible service instead of AWS, over HTTPS.
    #[serde(default)]
    pub(crate) endpoint: Option<Url>,
    /// Names the bucket in the request path instead of the host name.
    #[serde(default)]
    pub(crate) force_path_style: bool,
}

/// Why the S3 configuration cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum S3ConfigError {
    #[error("{0}")]
    Read(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

impl S3Config {
    /// The configuration in the JSON file at `path`, checked.
    pub(crate) async fn read(path: &Path) -> Result<Self, S3ConfigError> {
        let bytes = tokio::fs::read(path).await?;
        let config: Self = serde_json::from_slice(&bytes).map_err(|error| S3ConfigError::Invalid(error.to_string()))?;
        config.check()?;
        Ok(config)
    }

    /// Checks what the JSON's shape cannot: a bucket and a region, and an
    /// HTTPS endpoint.
    pub(crate) fn check(&self) -> Result<(), S3ConfigError> {
        if self.bucket.is_empty() || self.region.is_empty() {
            return Err(S3ConfigError::Invalid("bucket and region must not be empty".into()));
        }
        if self.endpoint.as_ref().is_some_and(|endpoint| endpoint.scheme() != "https") {
            return Err(S3ConfigError::Invalid("the endpoint must be an HTTPS URL".into()));
        }
        Ok(())
    }

    /// The bucket's client, which checksums every upload with SHA-256 and
    /// creates an object only if its key is free when asked to.
    pub(crate) fn builder(&self) -> AmazonS3Builder {
        let builder = AmazonS3Builder::from_env()
            .with_bucket_name(&self.bucket)
            .with_region(&self.region)
            .with_checksum_algorithm(Checksum::SHA256);
        match &self.endpoint {
            Some(endpoint) if self.force_path_style => builder
                .with_endpoint(endpoint.as_str().trim_end_matches('/'))
                .with_virtual_hosted_style_request(false),
            Some(endpoint) => {
                // A virtual-hosted endpoint names the bucket in its host.
                let mut hosted = endpoint.clone();
                let host = format!("{}.{}", self.bucket, endpoint.host_str().unwrap_or_default());
                // A URL that has a host takes another.
                let _ = hosted.set_host(Some(&host));
                builder
                    .with_endpoint(hosted.as_str().trim_end_matches('/'))
                    .with_virtual_hosted_style_request(true)
            }
            None => builder.with_virtual_hosted_style_request(!self.force_path_style),
        }
    }
}

/// The object store: the S3 bucket `s3` names, or the data directory.
pub(crate) async fn open(data_dir: &Path, s3: Option<&S3Config>) -> Result<Arc<dyn ObjectStore>, StorageError> {
    if let Some(config) = s3 {
        return Ok(Arc::new(config.builder().build()?));
    }
    let root = data_dir.to_owned();
    // Resolving the directory is file system work.
    let local = tokio::task::spawn_blocking(move || LocalFileSystem::new_with_prefix(root)).await??;
    Ok(Arc::new(local))
}

/// An S3-compatible service in memory, for the tests of what reaches the
/// object store: puts, conditional creation, reads with their metadata, and
/// deletes of one bucket's objects. It checks no signature.
#[cfg(test)]
pub(crate) mod fake_s3 {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::Bytes;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
    use axum::response::{IntoResponse, Response};
    use object_store::ClientOptions;
    use object_store::aws::AmazonS3;

    use super::S3Config;

    /// An object and the metadata it was put with.
    #[derive(Clone)]
    struct Object {
        bytes: Bytes,
        metadata: Vec<(HeaderName, HeaderValue)>,
    }

    /// The bucket's objects, and the keys of the objects created, in order.
    #[derive(Default)]
    struct Bucket {
        objects: HashMap<String, Object>,
        written: Vec<String>,
    }

    type Objects = Arc<Mutex<Bucket>>;

    /// A running fake, which stops when dropped.
    pub(crate) struct FakeS3 {
        pub(crate) endpoint: url::Url,
        objects: Objects,
        _serving: tokio::task::JoinHandle<()>,
    }

    impl Drop for FakeS3 {
        fn drop(&mut self) {
            self._serving.abort();
        }
    }

    impl FakeS3 {
        pub(crate) async fn start() -> Self {
            let objects = Objects::default();
            let app = Router::new()
                .route("/{bucket}/{*key}", axum::routing::any(object))
                .with_state(objects.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap()).parse().unwrap();
            let serving = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                endpoint,
                objects,
                _serving: serving,
            }
        }

        /// The fake's bucket as the backend's configuration names it.
        pub(crate) fn config(&self) -> S3Config {
            S3Config {
                bucket: "demi".into(),
                region: "us-east-1".into(),
                endpoint: Some(self.endpoint.clone()),
                force_path_style: true,
            }
        }

        /// The backend's client of the fake, which speaks plain HTTP to it.
        pub(crate) fn client(&self) -> AmazonS3 {
            self.config()
                .builder()
                .with_client_options(ClientOptions::new().with_allow_http(true))
                .with_access_key_id("fake")
                .with_secret_access_key("fake")
                .build()
                .unwrap()
        }

        /// The keys the bucket holds, sorted.
        pub(crate) fn keys(&self) -> Vec<String> {
            let mut keys: Vec<String> = self.objects.lock().unwrap().objects.keys().cloned().collect();
            keys.sort();
            keys
        }

        /// The keys of the objects put, in the order they were put.
        pub(crate) fn written(&self) -> Vec<String> {
            self.objects.lock().unwrap().written.clone()
        }

        /// The bytes of the object at `key`.
        pub(crate) fn object(&self, key: &str) -> Option<Bytes> {
            let bucket = self.objects.lock().unwrap();
            bucket.objects.get(key).map(|object| object.bytes.clone())
        }
    }

    fn error(status: StatusCode, code: &str) -> Response {
        let body = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{code}</Message></Error>");
        (status, [("content-type", "application/xml")], body).into_response()
    }

    async fn object(
        State(objects): State<Objects>,
        method: Method,
        Path((_bucket, key)): Path<(String, String)>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let etag = |bytes: &Bytes| format!("\"{:x}-{}\"", bytes.len(), bytes.first().copied().unwrap_or(0));
        match method {
            Method::PUT => {
                let mut bucket = objects.lock().unwrap();
                let create = headers.get("if-none-match").is_some_and(|value| value == "*");
                if create && bucket.objects.contains_key(&key) {
                    return error(StatusCode::PRECONDITION_FAILED, "PreconditionFailed");
                }
                let metadata = headers
                    .iter()
                    .filter(|(name, _)| name.as_str().starts_with("x-amz-meta-"))
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect();
                let tag = etag(&body);
                bucket.written.push(key.clone());
                bucket.objects.insert(key, Object { bytes: body, metadata });
                (StatusCode::OK, [("etag", tag)]).into_response()
            }
            Method::GET | Method::HEAD => {
                let Some(found) = objects.lock().unwrap().objects.get(&key).cloned() else {
                    return error(StatusCode::NOT_FOUND, "NoSuchKey");
                };
                let mut answer = HeaderMap::new();
                answer.insert("etag", HeaderValue::from_str(&etag(&found.bytes)).unwrap());
                answer.insert("last-modified", HeaderValue::from_static("Thu, 24 Sep 2026 08:00:00 GMT"));
                answer.insert("content-length", HeaderValue::from(found.bytes.len()));
                for (name, value) in found.metadata {
                    answer.insert(name, value);
                }
                if method == Method::HEAD {
                    (StatusCode::OK, answer).into_response()
                } else {
                    (StatusCode::OK, answer, found.bytes).into_response()
                }
            }
            Method::DELETE => {
                objects.lock().unwrap().objects.remove(&key);
                StatusCode::NO_CONTENT.into_response()
            }
            _ => error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use demi_core::CommandId;
    use demi_web_api::ids::{ConversationId, UserId};

    use super::fake_s3::FakeS3;
    use object_store::ObjectStore;

    use super::*;
    use crate::storage::blobs::BlobStores;
    use crate::storage::changes::ChangeStore;

    #[tokio::test]
    async fn an_s3_bucket_holds_the_blobs_once_and_the_change_store_like_the_data_directory() {
        let fake = FakeS3::start().await;
        let objects: Arc<dyn ObjectStore> = Arc::new(fake.client());
        let blobs = BlobStores::new(objects.clone()).for_user(&UserId::try_from("ana").unwrap());
        let first = blobs.put(Bytes::from_static(b"picture")).await.unwrap();
        // The same bytes again are the same blob, created once.
        let again = blobs.put(Bytes::from_static(b"picture")).await.unwrap();
        assert_eq!(first, again);
        assert_eq!(blobs.get(&first).await.unwrap(), Some(Bytes::from_static(b"picture")));

        let changes = ChangeStore::new(objects);
        let conversation = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01").unwrap();
        let command = CommandId::try_from("command-1").unwrap();
        let change = demi_runner_protocol::wire::JobFileChange {
            path: "/work/notes.md".into(),
            kind: demi_command_service::protocol::EditKind::Added,
            edits: vec![demi_command_service::protocol::EditCopies {
                original: None,
                modified: Some("copy".into()),
            }],
            added: 1,
            removed: 0,
        };
        let read = async |_: &str| Ok(Bytes::from_static(b"fresh\n"));
        let files = changes.retain(&conversation, &command, read, &[change]).await;
        assert!(files[0].edits[0].kept);
        let sides = changes
            .read(&conversation, &command, &files, "/work/notes.md", 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((sides.original.as_str(), sides.modified.as_str()), ("", "fresh\n"));
        let keys = fake.keys();
        assert!(keys.iter().any(|key| key.starts_with("blobs/ana/")), "{keys:?}");
        assert!(keys.contains(&format!("changes/{conversation}/command-1/0/0.modified")), "{keys:?}");
    }

    #[tokio::test]
    async fn the_s3_configuration_names_a_bucket_and_region_over_https() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("s3.json");
        let write = |text: &str| std::fs::write(&path, text).unwrap();
        write(r#"{"bucket":"demi","region":"eu-west-1","endpoint":"https://objects.example.com","forcePathStyle":true}"#);
        let config = S3Config::read(&path).await.unwrap();
        assert!(config.force_path_style);
        for refused in [
            r#"{"bucket":"demi","region":"eu-west-1","endpoint":"http://objects.example.com"}"#,
            r#"{"bucket":"","region":"eu-west-1"}"#,
            r#"{"bucket":"demi","region":"eu-west-1","profile":"default"}"#,
            r#"{"region":"eu-west-1"}"#,
        ] {
            write(refused);
            assert!(S3Config::read(&path).await.is_err(), "{refused}");
        }
    }
}
