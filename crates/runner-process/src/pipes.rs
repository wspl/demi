//! The pipe endpoints that carry file contents and output to the backend's
//! pipe routes, and the report of a pipe's outcome (`runner.md` § Pipes and
//! output). A content pipe from the runner is a `PUT` body; a stream pipe,
//! whose bytes must arrive as they are written, goes over a WebSocket at the
//! same path, since a TLS edge in front of the backend may hold a request's
//! body until it ends (`runner.md` § Host operations).

use bytes::Bytes;
use crate::backend::Backend;
use demi_runner_protocol::{values::DeviceToken, wire};
use futures_util::{SinkExt, Stream, StreamExt, stream::BoxStream};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{
    self, Message,
    client::IntoClientRequest,
    http::{HeaderValue, header::AUTHORIZATION},
    protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
};
use tokio_util::sync::CancellationToken;

/// How long a pipe's connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The most of a pipe's answer the runner reads: a confirmation, or the words
/// of a refusal.
const ANSWER_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct PipeClient {
    backend: Backend,
    http: reqwest::Client,
    origin: reqwest::Url,
    /// How long a connection may take to open, a stream pipe's WebSocket
    /// handshake included.
    connect_timeout: Duration,
    /// The registration's device token, which a claim can change.
    token: watch::Receiver<Option<DeviceToken>>,
}

impl PipeClient {
    pub fn new(
        backend: &Backend,
        token: watch::Receiver<Option<DeviceToken>>,
    ) -> io::Result<Self> {
        Self::with_connect_timeout(backend, token, CONNECT_TIMEOUT)
    }

    /// A client whose connections may take at most `connect_timeout` to
    /// open. Only opening has a deadline: a request, an answer or a body may
    /// stay quiet for as long as its pipe lasts. [`PipeClient::new`] gives a
    /// connection 15 seconds.
    pub fn with_connect_timeout(
        backend: &Backend,
        token: watch::Receiver<Option<DeviceToken>>,
        connect_timeout: Duration,
    ) -> io::Result<Self> {
        let origin = backend.origin()?;
        // Over TLS the edge's ALPN picks HTTP/2 when it offers it, and the
        // pipes share that one connection (`runner.md` § Host operations);
        // its windows grow with the link, so a file far away moves at the
        // link's pace.
        let http = backend
            .http(reqwest::Client::builder())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(connect_timeout)
            .http2_adaptive_window(true)
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            backend: backend.clone(),
            http,
            origin,
            connect_timeout,
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
            response = demi_command_sdk::descriptors::retry(&cancel, || async {
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
                let mut backoff = demi_command_sdk::descriptors::Backoff::default();
                let response = loop {
                    let request = self.request(reqwest::Method::PUT, path)?;
                    match request.body(reqwest::Body::wrap_stream(body.attempt())).send().await {
                        Ok(response) => break response,
                        // Out of open files, the connection waits for one
                        // (`runner.md` § Load). Only an attempt that read none
                        // of the body can be made again.
                        Err(error) if body.unread() && demi_command_sdk::descriptors::exhausted(&error) => {
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

    /// Sends `body` as a stream pipe: each chunk in binary messages as it
    /// comes, its end as the close frame, and its failure as a close frame
    /// with the error. The backend's close frame ends the pipe early: a
    /// normal one when its sink stopped taking bytes, another when the pipe
    /// failed. Once the body ended, the backend's close answers the runner's.
    pub async fn stream<S>(&self, path: &str, body: S, cancel: &CancellationToken) -> io::Result<()>
    where
        S: Stream<Item = io::Result<Bytes>> + Send + 'static,
    {
        let mut url = self.url(path)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| io::Error::other("invalid WebSocket scheme"))?;
        let mut request = url.as_str().into_client_request().map_err(io::Error::other)?;
        let authorization = HeaderValue::from_str(&format!("Bearer {}", self.token()?.expose()))
            .map_err(io::Error::other)?;
        request.headers_mut().insert(AUTHORIZATION, authorization);
        let config = WebSocketConfig::default()
            .max_message_size(Some(wire::STREAM_PIPE_MESSAGE_BYTES))
            .max_frame_size(Some(wire::STREAM_PIPE_MESSAGE_BYTES));
        // Out of open files, the connection waits for one (`runner.md` § Load).
        let connecting = demi_command_sdk::descriptors::retry(cancel, || async {
            self.backend
                .websocket(request.clone(), config)
                .await
                .map_err(socket_error)
        });
        let (socket, _) = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            connected = tokio::time::timeout(self.connect_timeout, connecting) => connected
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "stream pipe did not open"))??,
        };
        let (mut sender, mut receiver) = socket.split();
        let sending = async {
            tokio::pin!(body);
            while let Some(chunk) = body.next().await {
                let bytes = match chunk {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let frame = CloseFrame {
                            code: CloseCode::Error,
                            reason: wire::close_reason(&error.to_string()).into(),
                        };
                        // The pipe fails either way; a close that does not
                        // go out fails it as a lost connection.
                        let _closed = sender.send(Message::Close(Some(frame))).await;
                        return Err(error);
                    }
                };
                for start in (0..bytes.len()).step_by(wire::STREAM_PIPE_MESSAGE_BYTES) {
                    let end = (start + wire::STREAM_PIPE_MESSAGE_BYTES).min(bytes.len());
                    sender
                        .send(Message::Binary(bytes.slice(start..end)))
                        .await
                        .map_err(socket_error)?;
                }
            }
            let frame = CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            };
            sender
                .send(Message::Close(Some(frame)))
                .await
                .map_err(socket_error)
        };
        // The backend's close frame, which says how the pipe ended.
        let closed = async {
            while let Some(message) = receiver.next().await {
                if let Message::Close(frame) = message.map_err(socket_error)? {
                    return match frame {
                        None => Ok(()),
                        Some(frame) if frame.code == CloseCode::Normal => Ok(()),
                        Some(frame) => Err(io::Error::other(format!(
                            "pipe failed ({}): {}",
                            u16::from(frame.code),
                            frame.reason
                        ))),
                    };
                }
            }
            Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "stream pipe closed without a close frame",
            ))
        };
        tokio::pin!(closed);
        tokio::select! {
            _ = cancel.cancelled() => Err(cancelled()),
            outcome = &mut closed => outcome,
            sent = sending => {
                sent?;
                tokio::select! {
                    _ = cancel.cancelled() => Err(cancelled()),
                    outcome = closed => outcome,
                }
            }
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> io::Result<reqwest::RequestBuilder> {
        let url = self.url(path)?;
        let token = self.token()?;
        Ok(self.http.request(method, url).bearer_auth(token.expose()))
    }

    /// The backend's URL of the pipe at `path`, which must stay on its origin.
    fn url(&self, path: &str) -> io::Result<reqwest::Url> {
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
        Ok(url)
    }

    /// The registration's device token, which every pipe bears.
    fn token(&self) -> io::Result<DeviceToken> {
        self.token.borrow().clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runner has no device token",
            )
        })
    }
}

/// A stream pipe's WebSocket failure as an IO error: the system's own when
/// there is one, so running out of open files shows as such, and a refused
/// handshake with the backend's status and words.
fn socket_error(error: tungstenite::Error) -> io::Error {
    match error {
        tungstenite::Error::Io(error) => error,
        tungstenite::Error::Http(response) => {
            let body = response.body().as_deref().unwrap_or_default();
            let body = &body[..body.len().min(ANSWER_BYTES)];
            io::Error::other(format!(
                "pipe refused ({}): {}",
                response.status(),
                String::from_utf8_lossy(body)
            ))
        }
        error => io::Error::other(error),
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

/// Reports a pipe end's outcome to the backend; `shutdown` stops reporting
/// without a backend to receive it.
pub async fn report_pipe(
    output: &mpsc::Sender<wire::Frame>,
    id: String,
    result: io::Result<()>,
    shutdown: &CancellationToken,
) {
    match wire::encode(&wire::Outbound::PipeDone {
        pipe_id: id,
        ok: result.is_ok(),
        error: result.err().map(|error| error.to_string()),
    }) {
        Ok(message) => {
            tokio::select! {
                _ = shutdown.cancelled() => {},
                _ = output.send(message) => {},
            }
        }
        Err(error) => tracing::warn!("pipe result encoding failed: {error}"),
    }
}
