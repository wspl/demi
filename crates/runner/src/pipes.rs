use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream::BoxStream};
use std::{io, sync::Arc};
use demi_runner_protocol::values::{BackendUrl, DeviceToken};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// How long a pipe's connection may take to open.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// The most of a pipe's answer the runner reads: a confirmation, or the words
/// of a refusal.
const ANSWER_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct PipeClient {
    http: reqwest::Client,
    origin: reqwest::Url,
    /// The registration's device token, which a claim can change.
    token: watch::Receiver<Option<DeviceToken>>,
}

impl PipeClient {
    pub fn new(backend: &BackendUrl, token: watch::Receiver<Option<DeviceToken>>) -> io::Result<Self> {
        let mut origin = backend.url().clone();
        match origin.scheme() {
            "ws" => origin
                .set_scheme("http")
                .map_err(|_| io::Error::other("invalid HTTP origin"))?,
            "wss" => origin
                .set_scheme("https")
                .map_err(|_| io::Error::other("invalid HTTPS origin"))?,
            _ => {}
        }
        origin.set_path("/");
        origin.set_query(None);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            http,
            origin,
            token,
        })
    }

    pub async fn get(
        &self,
        path: &str,
        cancel: CancellationToken,
    ) -> io::Result<BoxStream<'static, io::Result<Bytes>>> {
        // Out of open files, the connection waits for one (`runner.md` § Load).
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            response = demi_command_service::descriptors::retry(&cancel, || async {
                let request = self.request(reqwest::Method::GET, path)?;
                request.send().await.map_err(io::Error::other)
            }) => expect_ok(response?).await?,
        };
        let stream = response
            .bytes_stream()
            .map(|result| result.map_err(io::Error::other));
        Ok(
            futures_util::stream::unfold(Some((Box::pin(stream), cancel)), |state| async move {
                let (mut stream, cancel) = state?;
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => Some((Err(cancelled()), None)),
                    chunk = stream.next() => chunk.map(|chunk| (chunk, Some((stream, cancel)))),
                }
            })
            .boxed(),
        )
    }

    pub async fn put<S>(&self, path: &str, body: S, cancel: &CancellationToken) -> io::Result<()>
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        let body = RetryableBody::new(body);
        tokio::select! {
            _ = cancel.cancelled() => Err(cancelled()),
            result = async {
                let mut backoff = demi_command_service::descriptors::Backoff::default();
                let response = loop {
                    let request = self.request(reqwest::Method::PUT, path)?;
                    match request.body(reqwest::Body::wrap_stream(body.attempt())).send().await {
                        Ok(response) => break response,
                        // Out of open files, the connection waits for one
                        // (`runner.md` § Load). Only an attempt that read none
                        // of the body can be made again.
                        Err(error) if body.unread() && demi_command_service::descriptors::exhausted(&error) => {
                            backoff.wait().await;
                        }
                        Err(error) => return Err(io::Error::other(error)),
                    }
                };
                let mut response = expect_ok(response).await?;
                // Consume the confirmation with a bound; a pipe upload has no payload response.
                let mut size = 0usize;
                while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
                    size += chunk.len();
                    if size > ANSWER_BYTES { return Err(io::Error::new(io::ErrorKind::InvalidData, "oversized pipe confirmation")); }
                }
                Ok(())
            } => result,
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> io::Result<reqwest::RequestBuilder> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pipe URL must be origin-relative",
            ));
        }
        let url = self.origin.join(path).map_err(io::Error::other)?;
        if url.origin() != self.origin.origin() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pipe URL changed backend origin",
            ));
        }
        let token = self.token.borrow().clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runner has no device token",
            )
        })?;
        Ok(self.http.request(method, url).bearer_auth(token.expose()))
    }
}

async fn expect_ok(mut response: reqwest::Response) -> io::Result<reqwest::Response> {
    if response.status() == reqwest::StatusCode::OK {
        return Ok(response);
    }
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
        let count = (ANSWER_BYTES - body.len()).min(chunk.len());
        body.extend_from_slice(&chunk[..count]);
        if body.len() == ANSWER_BYTES {
            break;
        }
    }
    Err(io::Error::other(format!(
        "pipe refused ({status}): {}",
        String::from_utf8_lossy(&body)
    )))
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "pipe cancelled")
}

/// A request body that connection attempts share: the first poll of an
/// attempt takes the body, so an attempt that failed before reading any of it
/// leaves it whole for the next, and one that read some leaves none.
struct RetryableBody<S>(Arc<Slot<S>>);

/// The body until an attempt takes it. A std mutex: hyper polls the body on
/// its connection's task while the retry loop checks the slot between
/// attempts, and the lock is held only to take the body, once per attempt.
type Slot<S> = std::sync::Mutex<Option<std::pin::Pin<Box<S>>>>;

struct BodyAttempt<S> {
    slot: Arc<Slot<S>>,
    body: Option<std::pin::Pin<Box<S>>>,
}

impl<S> RetryableBody<S> {
    fn new(body: S) -> Self {
        Self(Arc::new(std::sync::Mutex::new(Some(Box::pin(body)))))
    }

    fn attempt(&self) -> BodyAttempt<S> {
        BodyAttempt {
            slot: self.0.clone(),
            body: None,
        }
    }

    /// Whether no attempt has read the body yet.
    fn unread(&self) -> bool {
        self.0.lock().expect("the body slot is intact").is_some()
    }
}

impl<S: Stream<Item = io::Result<Bytes>>> Stream for BodyAttempt<S> {
    type Item = io::Result<Bytes>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let attempt = self.get_mut();
        if attempt.body.is_none() {
            attempt.body = attempt.slot.lock().expect("the body slot is intact").take();
        }
        match &mut attempt.body {
            Some(body) => body.as_mut().poll_next(context),
            // Attempts follow one another, and none follows one that read.
            None => std::task::Poll::Ready(None),
        }
    }
}
