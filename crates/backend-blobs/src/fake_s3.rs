//! An S3-compatible service in memory, for the tests of what reaches the object
//! store.
//!
//! It serves these requests on one bucket's objects:
//!
//! - Puts, and conditional creation.
//! - Reads, with their metadata.
//! - Listings.
//! - Deletes, one at a time or in a batch.
//!
//! It checks no signature. A test may hold its puts, to act while a write is
//! under way.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
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
    /// The number of the put that wrote it, which its listed time counts
    /// in milliseconds, so a later put lists as written later.
    put: usize,
}

/// The bucket's objects, the keys of the objects created, in order, and
/// the hold on its puts, while a test holds them.
#[derive(Default)]
struct Bucket {
    objects: HashMap<String, Object>,
    written: Vec<String>,
    hold: Option<Hold>,
}

/// The time a listing gives the object of put number `put`: a fixed
/// moment and as many milliseconds.
fn listed_time(put: usize) -> String {
    let (seconds, milliseconds) = (put / 1000, put % 1000);
    format!(
        "2026-09-24T08:{:02}:{:02}.{milliseconds:03}Z",
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Where each put reports its key as it arrives, and what it waits for.
#[derive(Clone)]
struct Hold {
    arrived: tokio::sync::mpsc::UnboundedSender<String>,
    released: tokio::sync::watch::Receiver<bool>,
}

/// The fake's puts held from [`FakeS3::hold_writes`] until
/// [`WriteHold::release`] or the drop; while it lives, it hears of each
/// put, held or not, as it arrives.
pub struct WriteHold {
    objects: Objects,
    arrived: tokio::sync::mpsc::UnboundedReceiver<String>,
    released: tokio::sync::watch::Sender<bool>,
}

impl WriteHold {
    /// The key of the next put, once it arrives.
    pub async fn arrived(&mut self) -> String {
        self.arrived.recv().await.expect("the fake holds its puts while the hold lives")
    }

    /// The keys of the puts that arrived and were not taken yet.
    pub fn more(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.arrived.try_recv().ok()).collect()
    }

    /// Lets the held puts and every later one through.
    pub fn release(&self) {
        self.released.send_replace(true);
    }
}

impl Drop for WriteHold {
    fn drop(&mut self) {
        self.objects.lock().unwrap().hold = None;
        self.release();
    }
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
            .route("/{bucket}", axum::routing::any(bucket))
            .route("/{bucket}/{*key}", axum::routing::any(object))
            .with_state(objects.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
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

    /// Holds every put from now on until the hold is released or dropped.
    pub fn hold_writes(&self) -> WriteHold {
        let (arrived, heard) = tokio::sync::mpsc::unbounded_channel();
        let (released, waits) = tokio::sync::watch::channel(false);
        self.objects.lock().unwrap().hold = Some(Hold {
            arrived,
            released: waits,
        });
        WriteHold {
            objects: self.objects.clone(),
            arrived: heard,
            released,
        }
    }
}

fn error(status: StatusCode, code: &str) -> Response {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{code}</Message></Error>"
    );
    (status, [("content-type", "application/xml")], body).into_response()
}

/// The bucket's own requests: a listing of the keys under a prefix, which
/// fits in one answer, and a batch delete.
async fn bucket(
    State(objects): State<Objects>,
    method: Method,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    match method {
        Method::GET if query.get("list-type").is_some_and(|kind| kind == "2") => {
            let prefix = query.get("prefix").map(String::as_str).unwrap_or_default();
            let bucket = objects.lock().unwrap();
            let listed: BTreeMap<_, _> = bucket
                .objects
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .collect();
            let contents: String = listed
                .into_iter()
                .map(|(key, object)| {
                    format!(
                        "<Contents><Key>{key}</Key><Size>{}</Size><LastModified>{}</LastModified></Contents>",
                        object.bytes.len(),
                        listed_time(object.put)
                    )
                })
                .collect();
            let body = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult>{contents}<IsTruncated>false</IsTruncated></ListBucketResult>"
            );
            (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
        }
        Method::POST if query.contains_key("delete") => {
            // The request names each key as `<Key>…</Key>`; the backend's
            // keys hold nothing XML escapes.
            let request = String::from_utf8_lossy(&body);
            let keys: Vec<&str> = request
                .split("<Key>")
                .skip(1)
                .filter_map(|rest| rest.split_once("</Key>").map(|(key, _)| key))
                .collect();
            let mut bucket = objects.lock().unwrap();
            let deleted: String = keys
                .into_iter()
                .map(|key| {
                    bucket.objects.remove(key);
                    format!("<Deleted><Key>{key}</Key></Deleted>")
                })
                .collect();
            let body = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><DeleteResult>{deleted}</DeleteResult>"
            );
            (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
        }
        _ => error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
    }
}

async fn object(
    State(objects): State<Objects>,
    method: Method,
    Path((_bucket, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let etag = |bytes: &Bytes| {
        format!(
            "\"{:x}-{}\"",
            bytes.len(),
            bytes.first().copied().unwrap_or(0)
        )
    };
    match method {
        Method::PUT => {
            let hold = objects.lock().unwrap().hold.clone();
            if let Some(mut hold) = hold {
                // A hold that went is no longer heard, and lets the put
                // through.
                let _ = hold.arrived.send(key.clone());
                let _ = hold.released.wait_for(|released| *released).await;
            }
            let mut bucket = objects.lock().unwrap();
            let create = headers
                .get("if-none-match")
                .is_some_and(|value| value == "*");
            if create && bucket.objects.contains_key(&key) {
                return error(StatusCode::PRECONDITION_FAILED, "PreconditionFailed");
            }
            let metadata = headers
                .iter()
                .filter(|(name, _)| {
                    name.as_str().starts_with("x-amz-meta-") || *name == "content-encoding"
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            let tag = etag(&body);
            bucket.written.push(key.clone());
            let put = bucket.written.len();
            bucket.objects.insert(
                key,
                Object {
                    bytes: body,
                    metadata,
                    put,
                },
            );
            (StatusCode::OK, [("etag", tag)]).into_response()
        }
        Method::GET | Method::HEAD => {
            let Some(found) = objects.lock().unwrap().objects.get(&key).cloned() else {
                return error(StatusCode::NOT_FOUND, "NoSuchKey");
            };
            let mut answer = HeaderMap::new();
            answer.insert("etag", HeaderValue::from_str(&etag(&found.bytes)).unwrap());
            answer.insert(
                "last-modified",
                HeaderValue::from_static("Thu, 24 Sep 2026 08:00:00 GMT"),
            );
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
