//! An S3-compatible service in memory, for the tests of what reaches the
//! object store: puts, conditional creation, reads with their metadata, and
//! deletes of one bucket's objects. It checks no signature.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use object_store::ClientOptions;
use object_store::aws::AmazonS3;

use crate::store::S3Config;

/// An object and the metadata it was put with, its content coding among
/// them.
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
pub struct FakeS3 {
    pub endpoint: url::Url,
    objects: Objects,
    _serving: tokio::task::JoinHandle<()>,
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self._serving.abort();
    }
}

impl FakeS3 {
    pub async fn start() -> Self {
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
    pub fn config(&self) -> S3Config {
        S3Config {
            bucket: "demi".into(),
            region: "us-east-1".into(),
            endpoint: Some(self.endpoint.clone()),
            force_path_style: true,
        }
    }

    /// The backend's client of the fake, which speaks plain HTTP to it.
    pub fn client(&self) -> AmazonS3 {
        self.config()
            .builder()
            .with_client_options(ClientOptions::new().with_allow_http(true))
            .with_access_key_id("fake")
            .with_secret_access_key("fake")
            .build()
            .unwrap()
    }

    /// The keys of the objects put, in the order they were put.
    pub fn written(&self) -> Vec<String> {
        self.objects.lock().unwrap().written.clone()
    }

    /// The bytes of the object at `key`.
    pub fn object(&self, key: &str) -> Option<Bytes> {
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
                .filter(|(name, _)| name.as_str().starts_with("x-amz-meta-") || *name == "content-encoding")
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
