//! The requests the backend makes for the user from this device, such as a
//! provider entry's that connects through it
//! (`crates/core/src/device_requests.rs`), for as long as
//! its connection lasts.
//!
//! The runner makes each request to its URL with its own HTTP client, the
//! method, headers and body as given, and reaches the URL's host as any
//! program on the device does: `127.0.0.1` is the device, and a name or an
//! address of its network is resolved and reached there. It streams the
//! answer back: its status and headers in `http_answered` and its body through
//! the request's answer pipe.
//!
//! The answer pipe opens before the request goes out, so the pipe is never
//! waiting for its source while a slow resident thinks. Its end is the
//! request's: when the backend ends the pipe, its caller stopped waiting, and
//! the runner abandons the request and closes its connection, before the
//! answer's head or after.

use std::collections::HashSet;
use std::error::Error as _;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};

use reqwest::Url;

use bytes::Bytes;
use demi_runner_process::pipes::{PipeClient, report_pipe};
use demi_runner_protocol::device_requests::{CONNECT_TIMEOUT, HttpHeader, MAX_HEADERS};
use demi_runner_protocol::wire::{self, Inbound, Outbound, PipeRef};
use futures_util::StreamExt as _;
use reqwest::header::{CONNECTION, HeaderMap, HeaderName, HeaderValue};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// The headers that belong to one connection, not to the request, which
/// the runner's own connection sets for itself; those a `Connection` header
/// names are such too.
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// The requests of one connection to the backend, which end with it.
pub struct DeviceRequests {
    http: reqwest::Client,
    pipes: PipeClient,
    control: mpsc::UnboundedSender<wire::Frame>,
    closed: CancellationToken,
    /// Each request's task: dropped with the connection, which aborts them.
    requests: JoinSet<()>,
}

/// One `http_request`.
struct Request {
    id: String,
    url: String,
    method: String,
    headers: Vec<HttpHeader>,
    body: Option<PipeRef>,
    answer: PipeRef,
}

impl DeviceRequests {
    pub fn new(pipes: PipeClient, control: mpsc::UnboundedSender<wire::Frame>, closed: CancellationToken) -> io::Result<Self> {
        // An address is reached as it is: no proxy, `localhost` is
        // 127.0.0.1 whatever the device resolves it to, and a redirect is
        // the answer, which the backend's client follows or not.
        let http = reqwest::Client::builder()
            .no_proxy()
            .resolve("localhost", SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            // Beyond what the relay carries, so an answer with a few more
            // fails in Demi's words; hyper's own limit is 100.
            .http1_max_headers(4 * MAX_HEADERS)
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            http,
            pipes,
            control,
            closed,
            requests: JoinSet::new(),
        })
    }

    /// Takes the backend's `http_request`: whether it was one.
    pub fn handle(&mut self, message: Inbound) -> Result<(), Inbound> {
        let Inbound::HttpRequest {
            id,
            url,
            method,
            headers,
            body,
            answer,
        } = message
        else {
            return Err(message);
        };
        // Finished requests leave nothing to keep.
        while self.requests.try_join_next().is_some() {}
        let request = Request {
            id,
            url,
            method,
            headers,
            body,
            answer,
        };
        let serving = Serving {
            http: self.http.clone(),
            pipes: self.pipes.clone(),
            control: self.control.clone(),
            closed: self.closed.clone(),
        };
        self.requests.spawn(serving.serve(request));
        Ok(())
    }
}

/// What one request's end line says of its answer.
#[derive(Default)]
struct Seen {
    status: AtomicU16,
    bytes: AtomicU64,
}

/// What one request's task uses.
struct Serving {
    http: reqwest::Client,
    pipes: PipeClient,
    control: mpsc::UnboundedSender<wire::Frame>,
    closed: CancellationToken,
}

/// The answer's body on its way to the answer pipe; an error ends the pipe
/// with it.
type Chunks = mpsc::Sender<io::Result<Bytes>>;

impl Serving {
    async fn serve(self, request: Request) {
        let cancel = self.closed.child_token();
        let began = std::time::Instant::now();
        let seen = Seen::default();
        let Request {
            id,
            url,
            method,
            headers,
            body,
            answer,
        } = request;
        // The answer pipe opens first and stays open while the address takes
        // its time, so only the caller decides how long that is. The body is
        // bulk, so each chunk waits for its taker rather than the runner
        // holding them without end.
        let (chunks, read) = mpsc::channel(1);
        let streamed = futures_util::stream::unfold(read, |mut read| async move {
            read.recv().await.map(|chunk| (chunk, read))
        });
        let piped = self.pipes.stream(&answer, streamed, &cancel);
        let exchange = self.exchange(&id, &url, &method, &headers, body.as_ref(), chunks, &cancel, &seen);
        tokio::pin!(piped);
        let mut exchange = Box::pin(exchange);
        let first = tokio::select! {
            sent = &mut piped => Err(sent),
            body_read = &mut exchange => Ok(body_read),
        };
        // The pipe ended first: the backend ended it, as it does when the
        // request's caller stopped waiting, or it failed. Dropping the
        // exchange abandons the request and closes its connection, whether
        // its answer had begun or not.
        drop(exchange);
        let (sent, body_read) = match first {
            Ok(body_read) => (piped.await, body_read),
            Err(sent) => {
                let ended = sent.as_ref().err().map_or_else(
                    || "the answer pipe ended before the answer".to_owned(),
                    ToString::to_string,
                );
                self.send(&Outbound::HttpFailed { id: id.clone(), message: ended });
                let abandoned = io::Error::new(io::ErrorKind::Interrupted, "the request was abandoned");
                (sent, Err(abandoned))
            }
        };
        // The host alone: a URL's path or query may hold a key.
        let host = Url::parse(&url).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
        tracing::info!(
            event = "device_request.ended",
            id = id.as_str(),
            host = host.as_str(),
            outcome = if sent.is_ok() && seen.status.load(Ordering::Relaxed) != 0 { "answered" } else { "failed" },
            status = seen.status.load(Ordering::Relaxed),
            bytes = seen.bytes.load(Ordering::Relaxed),
            ms = began.elapsed().as_millis() as u64,
            "a request the backend made from this device ended"
        );
        report_pipe(&self.control, answer.id.clone(), sent);
        if let Some(body) = body {
            report_pipe(&self.control, body.id, body_read);
        }
    }

    /// Makes the request and hands its answer on: its head in
    /// `http_answered`, its body to `chunks`; a request that got no answer
    /// the backend can take fails with `http_failed` and ends the pipe with
    /// why. Answers how reading the request's body went.
    async fn exchange(
        &self,
        id: &str,
        url: &str,
        method: &str,
        headers: &[HttpHeader],
        body: Option<&PipeRef>,
        chunks: Chunks,
        cancel: &CancellationToken,
        seen: &Seen,
    ) -> io::Result<()> {
        let (response, body_read) = match self.make(url, method, headers, body, cancel).await {
            Ok(made) => made,
            Err((error, body_read)) => {
                tracing::debug!(event = "device_request.failed", id, %error, "a request the backend made from this device failed");
                self.fail(id, error.to_string(), &chunks).await;
                return body_read;
            }
        };
        let answered = match answered(id, &response) {
            Ok(frame) => frame,
            Err(why) => {
                self.fail(id, why, &chunks).await;
                return body_read;
            }
        };
        seen.status.store(response.status().as_u16(), Ordering::Relaxed);
        if !self.send_frame(answered) {
            return body_read;
        }
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let ended = chunk.is_err();
            if let Ok(bytes) = &chunk {
                seen.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            }
            if chunks.send(chunk.map_err(io::Error::other)).await.is_err() || ended {
                break;
            }
        }
        body_read
    }

    /// Fails the request with `why`: in `http_failed`, and as the end of its
    /// answer pipe, so neither waits for an answer that never comes.
    async fn fail(&self, id: &str, why: String, chunks: &Chunks) {
        self.send(&Outbound::HttpFailed {
            id: id.to_owned(),
            message: why.clone(),
        });
        // The pipe ends either way: it ended already when nothing takes this.
        let _ = chunks.send(Err(io::Error::other(why))).await;
    }

    /// Makes the request: its answer's head, and how reading its body
    /// went; or why there is no answer.
    async fn make(
        &self,
        url: &str,
        method: &str,
        headers: &[HttpHeader],
        body: Option<&PipeRef>,
        cancel: &CancellationToken,
    ) -> Result<(reqwest::Response, io::Result<()>), (io::Error, io::Result<()>)> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| (io::Error::other(format!("{method} is not an HTTP method")), Ok(())))?;
        let mut map = HeaderMap::new();
        for header in headers {
            let name = HeaderName::from_bytes(header.name.as_bytes())
                .map_err(|_| (io::Error::other(format!("{} is not a header name", header.name)), Ok(())))?;
            let value = HeaderValue::from_str(&header.value)
                .map_err(|_| (io::Error::other(format!("the {name} header's value is not one")), Ok(())))?;
            map.append(name, value);
        }
        let map = end_to_end(&map);
        let mut request = self.http.request(method, url).headers(map);
        if let Some(body) = body {
            let read = match self.pipes.get(&body, cancel.clone()).await {
                Ok(read) => read,
                Err(error) => {
                    let failed = io::Error::new(error.kind(), error.to_string());
                    return Err((failed, Err(error)));
                }
            };
            request = request.body(reqwest::Body::wrap_stream(read));
        }
        let response = tokio::select! {
            () = cancel.cancelled() => {
                let ended = || io::Error::new(io::ErrorKind::Interrupted, "the connection ended");
                return Err((ended(), Err(ended())));
            }
            response = request.send() => response,
        };
        match response {
            Ok(response) => Ok((response, Ok(()))),
            Err(error) => Err((io::Error::other(causes(&error)), Ok(()))),
        }
    }

    /// Sends `message`; false once the connection's writer ended. The runner's own
    /// messages always encode.
    fn send(&self, message: &Outbound) -> bool {
        match wire::encode(message) {
            Ok(frame) => self.send_frame(frame),
            Err(error) => {
                tracing::warn!(event = "reply.encode_failed", what = "device_request", %error, "a device request's message was not encoded");
                true
            }
        }
    }

    fn send_frame(&self, frame: wire::Frame) -> bool {
        self.control.send(frame).is_ok()
    }
}

/// The `http_answered` of `response`, without its hop-by-hop headers; or
/// why the relay cannot carry it, such as more headers than it takes.
fn answered(id: &str, response: &reqwest::Response) -> Result<wire::Frame, String> {
    let headers: Vec<HttpHeader> = end_to_end(response.headers())
        .iter()
        // A value that is not text is not carried (`HttpHeader`).
        .filter_map(|(name, value)| {
            Some(HttpHeader {
                name: name.as_str().to_owned(),
                value: value.to_str().ok()?.to_owned(),
            })
        })
        .collect();
    if headers.len() > MAX_HEADERS {
        return Err(format!(
            "The answer has {} headers, more than the {MAX_HEADERS} Demi carries",
            headers.len()
        ));
    }
    let message = Outbound::HttpAnswered {
        id: id.to_owned(),
        status: response.status().as_u16(),
        headers,
    };
    let frame = wire::encode(&message).map_err(|error| format!("The answer cannot be carried: {error}"))?;
    wire::within_limit(frame, |why| Err(wire::WireError::Invalid(why)))
        .map_err(|error| format!("The answer's head cannot be carried: {error}"))
}

/// `headers` without those of one connection: the hop-by-hop ones and every
/// header a `Connection` header names.
fn end_to_end(headers: &HeaderMap) -> HeaderMap {
    let named: HashSet<String> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    let mut kept = HeaderMap::new();
    for (name, value) in headers {
        if !HOP_BY_HOP.contains(&name.as_str()) && !named.contains(name.as_str()) {
            kept.append(name.clone(), value.clone());
        }
    }
    kept
}

/// A failed request in words: the error and each of its causes, without
/// the URL, whose path or query may hold a key.
fn causes(error: &reqwest::Error) -> String {
    let mut message = if error.is_connect() {
        "The address could not be connected to".to_owned()
    } else if error.is_timeout() {
        "The address did not answer in time".to_owned()
    } else {
        "The request could not be made".to_owned()
    };
    let mut cause = error.source();
    while let Some(inner) = cause {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        cause = inner.source();
    }
    message
}
