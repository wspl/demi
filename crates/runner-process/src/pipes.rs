//! The pipe endpoints that carry file contents and output to the backend's
//! pipe routes, and the report of a pipe's outcome (`crates/runner-protocol/src/wire/pipe_frames.rs`). A content pipe from the runner is a `PUT` body; a stream pipe,
//! whose bytes must arrive as they are written, goes over a WebSocket at the
//! same path, since a TLS edge in front of the backend may hold a request's
//! body until it ends (`crates/runner-protocol/src/wire/pipe_frames.rs`).

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
/// How long a pipe request that fails on its connection before the backend
/// received any of it is tried again on new connections, as a download's
/// connections are (`native-runtime.md` § Downloads): a link through a
/// proxy that fails a request now and then must not end the pipe.
const RECONNECT_FOR: Duration = Duration::from_secs(60);
/// The pause between two such attempts.
const RECONNECT_PAUSE: Duration = Duration::from_secs(1);

/// The most of a pipe's answer the runner reads: a confirmation, or the words
/// of a refusal.
const ANSWER_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct PipeClient {
    backend: Backend,
    http: reqwest::Client,
    /// The client of the files the backend offers for download, which
    /// keeps their bytes as served (`native-runtime.md` § Downloads).
    downloads: reqwest::Client,
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
        // pipes share that one connection (`crates/runner-protocol/src/wire/pipe_frames.rs`);
        // its windows grow with the link, so a file far away moves at the
        // link's pace.
        let http = backend
            .http(reqwest::Client::builder())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(connect_timeout)
            .http2_adaptive_window(true)
            .build()
            .map_err(io::Error::other)?;
        let downloads = demi_shared_artifacts::download_client_through(|builder| backend.http(builder))
            .map_err(io::Error::other)?;
        Ok(Self {
            backend: backend.clone(),
            http,
            downloads,
            origin,
            connect_timeout,
            token,
        })
    }

    /// Reads the pipe at `path`: the bytes of its data frames, until its end
    /// frame (`crates/runner-protocol/src/wire/pipe_frames.rs`). An end frame with a cause is
    /// that cause's error; a body that stops without an end frame is a
    /// broken pipe; idle frames pass nothing on.
    pub async fn get(
        &self,
        path: &str,
        cancel: CancellationToken,
    ) -> io::Result<BoxStream<'static, io::Result<Bytes>>> {
        // Out of open files, the connection waits for one (`crates/command-sdk/src/descriptors.rs`).
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            response = reconnecting(|| demi_command_sdk::descriptors::retry(&cancel, || async {
                let request = self.request(reqwest::Method::GET, path).map_err(Attempt::Refused)?;
                request.send().await.map_err(Attempt::of)
            })) => expect_ok(response?).await?,
        };
        let reading = Reading {
            body: response.bytes_stream().boxed(),
            frames: wire::PipeFrameDecoder::new(),
            cancel,
        };
        Ok(futures_util::stream::unfold(Some(reading), |reading| async move {
            let mut reading = reading?;
            match reading.next_data().await {
                Ok(Some(bytes)) => Some((Ok(bytes), Some(reading))),
                Ok(None) => None,
                Err(error) => Some((Err(error), None)),
            }
        })
        .boxed())
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
                let mut lost_since = None;
                let response = loop {
                    let request = self.request(reqwest::Method::PUT, path)?;
                    match request.body(reqwest::Body::wrap_stream(body.attempt())).send().await {
                        Ok(response) => break response,
                        // Out of open files, the connection waits for one
                        // (`crates/command-sdk/src/descriptors.rs`). Only an attempt that read none
                        // of the body can be made again.
                        Err(error) if body.unread() && demi_command_sdk::descriptors::exhausted(&error) => {
                            backoff.wait().await;
                        }
                        // A connection that failed before the body went is
                        // tried again on a new one.
                        Err(error) if body.unread() && lost_connection(&error) => {
                            let since = *lost_since.get_or_insert_with(tokio::time::Instant::now);
                            if since.elapsed() >= RECONNECT_FOR {
                                return Err(io::Error::other(error));
                            }
                            tokio::time::sleep(RECONNECT_PAUSE).await;
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
        // Out of open files, the connection waits for one (`crates/command-sdk/src/descriptors.rs`).
        let connecting = reconnecting(|| async {
            let connecting = demi_command_sdk::descriptors::retry(cancel, || async {
                self.backend
                    .websocket(request.clone(), config)
                    .await
                    .map_err(|error| match error {
                        // A refused handshake reached the backend.
                        tungstenite::Error::Http(_) => Attempt::Refused(socket_error(error)),
                        error => Attempt::Lost(socket_error(error)),
                    })
            });
            tokio::time::timeout(self.connect_timeout, connecting)
                .await
                .unwrap_or_else(|_| Err(Attempt::Lost(io::Error::new(io::ErrorKind::TimedOut, "stream pipe did not open"))))
        });
        let (socket, _) = tokio::select! {
            _ = cancel.cancelled() => return Err(cancelled()),
            connected = connecting => connected?,
        };
        let (mut sender, mut receiver) = socket.split();
        let sending = async {
            tokio::pin!(body);
            loop {
                // A ping after 30 s without a message keeps the socket
                // through an edge that ends a silent connection, as a still
                // page's live view is (`crates/runner-protocol/src/wire/pipe_frames.rs`).
                let chunk = tokio::select! {
                    chunk = body.next() => chunk,
                    () = tokio::time::sleep(wire::PIPE_IDLE) => {
                        sender
                            .send(Message::Ping(Bytes::new()))
                            .await
                            .map_err(socket_error)?;
                        continue;
                    }
                };
                let Some(chunk) = chunk else {
                    break;
                };
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
                        // Why the pipe failed, in the backend's words.
                        Some(frame) if frame.code == CloseCode::Error => {
                            Err(io::Error::other(frame.reason.to_string()))
                        }
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

    /// Downloads the file the backend offers as `offer` into `staging`,
    /// resumed after a break or a stall (`native-runtime.md` § Downloads),
    /// then writes its bytes, checked against the offer's size and SHA-256,
    /// into `output`. The staging file is removed either way: a later call
    /// is given a new offer. `progress` hears how many bytes the staging
    /// file holds after each piece. The error names the cause beneath.
    pub async fn download(
        &self,
        offer: &wire::FileOffer,
        staging: &std::path::Path,
        output: &mut (impl tokio::io::AsyncWrite + Unpin),
        cancel: &CancellationToken,
        mut progress: impl FnMut(u64),
    ) -> io::Result<()> {
        let url = self.url(&offer.url)?;
        let token = self.token()?;
        let authorization = HeaderValue::from_str(&format!("Bearer {}", token.expose()))
            .map_err(io::Error::other)?;
        let origin = self.origin.origin();
        // The device token goes only to the backend's origin.
        let headers = move |url: &reqwest::Url| {
            let mut headers = reqwest::header::HeaderMap::new();
            if url.origin() == origin {
                headers.insert(AUTHORIZATION, authorization.clone());
            }
            headers
        };
        let download = demi_shared_artifacts::Download {
            limit: offer.size,
            headers: &headers,
            ..demi_shared_artifacts::Download::new(url.as_str())
        };
        let expected = demi_shared_artifacts::Digest {
            size: offer.size,
            sha256: offer.sha256.clone(),
        };
        let written = async {
            let received = demi_shared_artifacts::download(&self.downloads, download, staging, cancel, |held, _| progress(held)).await?;
            received.verify(staging, &expected, output, cancel).await
        }
        .await;
        demi_shared_artifacts::discard(staging).await;
        written.map_err(|error| match error {
            demi_shared_artifacts::Error::Io(error) => error,
            error => io::Error::other(error.to_string()),
        })
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

/// A pipe's response being read: its body and the frames read of it so far.
struct Reading {
    body: BoxStream<'static, reqwest::Result<Bytes>>,
    frames: wire::PipeFrameDecoder,
    cancel: CancellationToken,
}

impl Reading {
    /// The next data frame's bytes; none at a whole pipe's end.
    async fn next_data(&mut self) -> io::Result<Option<Bytes>> {
        loop {
            match self.frames.next_frame().map_err(io::Error::other)? {
                Some(wire::PipeFrame::Data(bytes)) => return Ok(Some(bytes)),
                Some(wire::PipeFrame::Idle) => continue,
                Some(wire::PipeFrame::End(Ok(()))) => return Ok(None),
                // The cause, in the words the backend gave it, stands alone
                // (`commands.md` § Handle an rpc call).
                Some(wire::PipeFrame::End(Err(cause))) => return Err(io::Error::other(cause)),
                None => {}
            }
            let chunk = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Err(cancelled()),
                chunk = self.body.next() => chunk,
            };
            match chunk {
                Some(Ok(bytes)) => self.frames.extend(&bytes),
                Some(Err(error)) => return Err(broken(&error.to_string())),
                None if self.frames.is_partial() => {
                    return Err(broken("its response ended within a frame"));
                }
                None => return Err(broken("its response ended without an end frame")),
            }
        }
    }
}

/// A pipe whose response stopped before its end frame, for `cause`.
fn broken(cause: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        format!("the pipe broke off before its end: {cause}"),
    )
}

/// Why one connection's attempt at a pipe request failed: on the way, so
/// that a new connection may succeed, or for a reason another attempt would
/// meet again, such as the backend's refusal.
#[derive(Debug)]
enum Attempt {
    Lost(io::Error),
    Refused(io::Error),
}

impl Attempt {
    fn of(error: reqwest::Error) -> Self {
        if lost_connection(&error) {
            Self::Lost(io::Error::other(error))
        } else {
            Self::Refused(io::Error::other(error))
        }
    }

    fn into_io(self) -> io::Error {
        match self {
            Self::Lost(error) | Self::Refused(error) => error,
        }
    }
}

impl std::fmt::Display for Attempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lost(error) | Self::Refused(error) => error.fmt(formatter),
        }
    }
}

/// The IO error beneath, so that running out of open files is found.
impl std::error::Error for Attempt {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lost(error) | Self::Refused(error) => Some(error),
        }
    }
}

/// Whether a request failed on its connection without an answer, as through
/// a proxy that drops a request now and then: a new connection may succeed.
fn lost_connection(error: &reqwest::Error) -> bool {
    !error.is_builder() && !error.is_redirect() && !error.is_status() && !error.is_decode()
}

/// Runs `attempt`, a pipe request that sends nothing the backend could have
/// received before it is answered, again on a new connection after one is
/// lost, for up to [`RECONNECT_FOR`]; the last failure is the request's.
async fn reconnecting<T, F, Fut>(mut attempt: F) -> io::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Attempt>>,
{
    let started = tokio::time::Instant::now();
    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(Attempt::Lost(error)) if started.elapsed() < RECONNECT_FOR => {
                tracing::debug!("a pipe request lost its connection and goes again: {error}");
                tokio::time::sleep(RECONNECT_PAUSE).await;
            }
            Err(error) => return Err(error.into_io()),
        }
    }
}

/// A stream pipe's WebSocket failure as an IO error: the system's own when
/// there is one, so running out of open files shows as such, and a refused
/// handshake as [`refused`] says.
fn socket_error(error: tungstenite::Error) -> io::Error {
    match error {
        tungstenite::Error::Io(error) => error,
        tungstenite::Error::Http(response) => {
            let body = response.body().as_deref().unwrap_or_default();
            refused(response.status(), &body[..body.len().min(ANSWER_BYTES)])
        }
        error => io::Error::other(error),
    }
}

/// The backend's refusal of a pipe request, with status `status` and words
/// `body`, as an IO error. A 409 says why the pipe cannot go on, such as
/// `build-box is offline: …`, which the command reports as its own error, so
/// its words stand alone (`commands.md` § Handle an rpc call); another status
/// is a fault of the request, named with the status.
fn refused(status: reqwest::StatusCode, body: &[u8]) -> io::Error {
    let words = String::from_utf8_lossy(body);
    if status == reqwest::StatusCode::CONFLICT {
        return io::Error::other(words.into_owned());
    }
    io::Error::other(format!("pipe refused ({status}): {words}"))
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
    Err(refused(status, &body))
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
