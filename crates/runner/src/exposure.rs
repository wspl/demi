//! The requests the backend makes for the user to this device's exposed
//! ports (`port-exposure.md`), for as long as its connection lasts: the
//! runner makes each to `127.0.0.1:<port>`, or `https://localhost:<port>`
//! connected to 127.0.0.1, with its own HTTP client, the method, path,
//! headers and body as given, and streams the answer back, its status and
//! headers in `http_answered` and its body through the request's answer
//! pipe. Which ports are exposed is the backend's to decide: it sends no
//! request to any other.
//!
//! The answer pipe opens before the request goes out, so the pipe is never
//! waiting for its source while a slow service thinks, and its end is the
//! request's: when the backend ends the pipe, its caller stopped waiting,
//! the runner abandons the request and closes its connection to the port,
//! before the answer's head or after.

use std::collections::HashSet;
use std::error::Error as _;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};

use bytes::Bytes;
use demi_runner_process::pipes::{PipeClient, report_pipe};
use demi_runner_protocol::exposure::{CONNECT_TIMEOUT, HttpHeader, HttpScheme, MAX_HEADERS};
use demi_runner_protocol::wire::{self, Inbound, Outbound, PipeRef};
use futures_util::StreamExt as _;
use reqwest::header::{CONNECTION, HeaderMap, HeaderName, HeaderValue};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// The headers that belong to one connection, not to the request, which
/// the runner's own connection to the port sets for itself; those a
/// `Connection` header names are such too.
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
pub struct Exposure {
    http: reqwest::Client,
    pipes: PipeClient,
    control: mpsc::Sender<wire::Frame>,
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

impl Exposure {
    pub fn new(pipes: PipeClient, control: mpsc::Sender<wire::Frame>, closed: CancellationToken) -> io::Result<Self> {
        // A port of this device is reached as it is: no proxy, `localhost`
        // is 127.0.0.1 whatever the device resolves it to, and a redirect
        // is the answer, which the backend's client follows or not.
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
            scheme,
            port,
            method,
            path,
            headers,
            body,
            answer,
        } = message
        else {
            return Err(message);
        };
        let url = match scheme {
            HttpScheme::Http => format!("http://127.0.0.1:{port}{path}"),
            HttpScheme::Https => format!("https://localhost:{port}{path}"),
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

/// What one request's task uses.
struct Serving {
    http: reqwest::Client,
    pipes: PipeClient,
    control: mpsc::Sender<wire::Frame>,
    closed: CancellationToken,
}

/// The answer's body on its way to the answer pipe; an error ends the pipe
/// with it.
type Chunks = mpsc::Sender<io::Result<Bytes>>;

impl Serving {
    async fn serve(self, request: Request) {
        let cancel = self.closed.child_token();
        let Request {
            id,
            url,
            method,
            headers,
            body,
            answer,
        } = request;
        // The answer pipe opens first and stays open while the port takes
        // its time, so only the caller decides how long that is.
        let (chunks, read) = mpsc::channel(1);
        let streamed = futures_util::stream::unfold(read, |mut read| async move {
            read.recv().await.map(|chunk| (chunk, read))
        });
        let piped = self.pipes.stream(&answer.url, streamed, &cancel);
        let exchange = self.exchange(&id, &url, &method, &headers, body.as_ref(), chunks, &cancel);
        tokio::pin!(piped);
        let mut exchange = Box::pin(exchange);
        let first = tokio::select! {
            sent = &mut piped => Err(sent),
            body_read = &mut exchange => Ok(body_read),
        };
        // The pipe ended first: the backend ended it, as it does when the
        // request's caller stopped waiting, or it failed. Dropping the
        // exchange abandons the request and closes its connection to the
        // port, whether its answer had begun or not.
        drop(exchange);
        let (sent, body_read) = match first {
            Ok(body_read) => (piped.await, body_read),
            Err(sent) => {
                let ended = sent.as_ref().err().map_or_else(
                    || "the answer pipe ended before the answer".to_owned(),
                    ToString::to_string,
                );
                self.send(&Outbound::HttpFailed { id, message: ended }).await;
                let abandoned = io::Error::new(io::ErrorKind::Interrupted, "the request was abandoned");
                (sent, Err(abandoned))
            }
        };
        report_pipe(&self.control, answer.id, sent, &self.closed).await;
        if let Some(body) = body {
            report_pipe(&self.control, body.id, body_read, &self.closed).await;
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
    ) -> io::Result<()> {
        let (response, body_read) = match self.make(url, method, headers, body, cancel).await {
            Ok(made) => made,
            Err((error, body_read)) => {
                tracing::debug!("a request to an exposed port failed: {error}");
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
        if !self.send_frame(answered).await {
            return body_read;
        }
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let ended = chunk.is_err();
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
        })
        .await;
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
            let read = match self.pipes.get(&body.url, cancel.clone()).await {
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

    /// Sends `message`; false once the connection closed. The runner's own
    /// messages always encode.
    async fn send(&self, message: &Outbound) -> bool {
        match wire::encode(message) {
            Ok(frame) => self.send_frame(frame).await,
            Err(error) => {
                tracing::warn!("an exposed port's message could not be encoded: {error}");
                true
            }
        }
    }

    async fn send_frame(&self, frame: wire::Frame) -> bool {
        tokio::select! {
            () = self.closed.cancelled() => false,
            sent = self.control.send(frame) => sent.is_ok(),
        }
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
/// the URL, which names this device's own address.
fn causes(error: &reqwest::Error) -> String {
    let mut message = if error.is_connect() {
        "The port refused the connection".to_owned()
    } else if error.is_timeout() {
        "The port did not answer in time".to_owned()
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
