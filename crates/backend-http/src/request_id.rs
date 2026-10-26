//! Request IDs.
//!
//! The edge names every request as it arrives, a WebSocket's upgrade included,
//! and never takes a name from the client.
//!
//! - The ID is a field of the request's `tracing` span.
//! - Every response carries it in `X-Request-Id`.
//! - Every JSON error carries it as `requestId`.
//! - Every request logs one line as it is answered, or as its client abandons
//!   it, so that an ID a user copies always finds its request.
//! - The page that made a request names its load in `X-Demi-Page`, or in a
//!   socket's `page` parameter; the span carries it as `page_id`, so the
//!   backend's lines of a request the page made are found by the page's ID
//!   beside the page's own lines.
//!
//! Work the request starts in another task is started inside its span: a
//! session it opens, a Host operation it sends and a store write it waits for
//! log under the same ID.
//!
//! A socket keeps the ID of its upgrade for its whole life. Work that outlives
//! a request, such as a turn a socket's frame started, logs under the socket's
//! ID with its own conversation and turn IDs. What an abandoned request started
//! goes on or stops by that work's own rule; its line only records it.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use data_encoding::{Encoding, Specification};
use demi_shared_types::RequestId;
use demi_web_api_protocol::diagnostics::PAGE_HEADER;
use demi_web_api_protocol::error::ErrorCode;

/// The header every response names its request in.
const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// The request ID Cloudflare gives each request it passes on, which the
/// request's span carries beside the backend's own.
pub(crate) const CF_RAY: &str = "cf-ray";

/// Lowercase Crockford base-32: no `i`, `l`, `o` or `u`, so an ID a user
/// reads out or types stays the same.
static ALPHABET: LazyLock<Encoding> = LazyLock::new(|| {
    let mut specification = Specification::new();
    specification
        .symbols
        .push_str("0123456789abcdefghjkmnpqrstvwxyz");
    specification
        .encoding()
        .expect("the lowercase Crockford alphabet is a valid base32")
});

tokio::task_local! {
    /// The ID of the request whose answer the task is building.
    static CURRENT: RequestId;
}

/// A new request's ID: `req_` and 16 characters, 80 random bits.
pub(crate) fn generate() -> RequestId {
    let bits: [u8; 10] = rand::random();
    RequestId::try_from(format!("req_{}", ALPHABET.encode(&bits)))
        .expect("a request ID is not empty")
}

/// The ID of the request being answered; none outside the edge's
/// [`assign`], as in a unit test that calls a handler.
pub(crate) fn current() -> Option<RequestId> {
    CURRENT.try_with(Clone::clone).ok()
}

/// The ID of the request being answered; outside the edge's [`assign`], as
/// in a unit test that calls a handler, an ID no request had.
pub(crate) fn current_or_new() -> RequestId {
    current().unwrap_or_else(generate)
}

/// Names the request: its extensions carry the ID for the span and the
/// handlers, the answer is built where [`current`] reads it, and the
/// response carries it in `X-Request-Id`.
pub(crate) async fn assign(mut request: Request, next: Next) -> Response {
    let id = generate();
    request.extensions_mut().insert(id.clone());
    let mut response = CURRENT.scope(id.clone(), next.run(request)).await;
    response.headers_mut().insert(
        X_REQUEST_ID,
        HeaderValue::from_str(id.as_str()).expect("a request ID is visible ASCII"),
    );
    response
}

/// Whether `text` is a page load's ID: `pg_` and 16 characters of the
/// request ID's alphabet. A value of any other shape is not taken, since it
/// goes into the log as an ID.
fn is_page_id(text: &str) -> bool {
    text.strip_prefix("pg_").is_some_and(|rest| {
        rest.len() == 16 && rest.bytes().all(|byte| ALPHABET_SYMBOLS.contains(&byte))
    })
}

const ALPHABET_SYMBOLS: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The page load the request came from: its `X-Demi-Page`, or the `page`
/// parameter of a socket's URL, which is the only part of the query read.
fn page_of<B>(request: &axum::http::Request<B>) -> Option<&str> {
    let header = request
        .headers()
        .get(PAGE_HEADER)
        .and_then(|value| value.to_str().ok());
    let parameter = || {
        request
            .uri()
            .query()?
            .split('&')
            .find_map(|pair| pair.strip_prefix(concat!("page", "=")))
    };
    header.or_else(parameter).filter(|page| is_page_id(page))
}

/// The span every event of `request` is logged in: its ID, method and path,
/// never its query, which can carry a file's path or a token, the page load
/// that made it and Cloudflare's `cf-ray` when a proxy passes it. The session
/// gate records the caller as `user_id`, and the lookup of the conversation or
/// the device a route names records it as `conversation_id` or `host_id`.
pub(crate) fn span<B>(request: &axum::http::Request<B>) -> tracing::Span {
    let id = request.extensions().get::<RequestId>();
    let ray = request
        .headers()
        .get(CF_RAY)
        .and_then(|value| value.to_str().ok());
    tracing::info_span!(
        "request",
        request_id = id.map(tracing::field::display),
        method = %request.method(),
        path = logged_path(request.uri().path()).as_ref(),
        cf_ray = ray,
        page_id = page_of(request),
        user_id = tracing::field::Empty,
        conversation_id = tracing::field::Empty,
        host_id = tracing::field::Empty,
    )
}

/// The path as the log holds it: a token in a path, the key to a share's link
/// or to an HTML view, is the segment after `shares` or `html` and never
/// reaches a line.
fn logged_path(path: &str) -> std::borrow::Cow<'_, str> {
    let keyed = |segment: &str| segment == "shares" || segment == "html";
    let mut segments = path.split('/');
    let mut after_key = false;
    if !segments.any(|segment| std::mem::replace(&mut after_key, keyed(segment))) {
        return std::borrow::Cow::Borrowed(path);
    }
    let mut hidden = false;
    let redacted: Vec<&str> = path
        .split('/')
        .map(|segment| {
            let secret = std::mem::replace(&mut hidden, keyed(segment));
            if secret { "{token}" } else { segment }
        })
        .collect();
    std::borrow::Cow::Owned(redacted.join("/"))
}

/// How long an abandoned request runs before its line is a warning.
const SLOW_ABANDONED: Duration = Duration::from_secs(10);

/// Logs *a request was abandoned* for a request whose client went away before
/// its answer, a closed tab or a timed-out fetch.
///
/// Hyper then drops the answer's future, and with it this middleware's guard,
/// which no answer released. Runs inside the request's span, which it logs in,
/// so the line carries the span's ID, method, path, user and `cf-ray`. The work
/// the request started goes on or stops as that work's own rule says.
pub(crate) async fn watch_abandon(request: Request, next: Next) -> Response {
    let mut pending = Pending {
        span: tracing::Span::current(),
        started: Instant::now(),
        answered: false,
    };
    let response = next.run(request).await;
    pending.answered = true;
    response
}

/// A request not answered yet; dropped unanswered, it logs its abandon.
struct Pending {
    span: tracing::Span,
    started: Instant,
    answered: bool,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        let ran = self.started.elapsed();
        let duration_ms = u64::try_from(ran.as_millis()).unwrap_or(u64::MAX);
        self.span.in_scope(|| {
            if ran > SLOW_ABANDONED {
                tracing::warn!(event = "request.abandoned", duration_ms, "a request was abandoned");
            } else {
                tracing::info!(event = "request.abandoned", duration_ms, "a request was abandoned");
            }
        });
    }
}

/// The line a request logs as it is answered, in its span: the status, how
/// long it took, the `code` of a JSON error, the answer's `bytes` when its
/// length is known, whether it was a 304 and its `content_encoding`, at `info`, and at `warn` for
/// a 5xx answer. An upgrade's answer logs nothing: its socket's end line is
/// its line.
pub(crate) fn answered<B>(response: &axum::http::Response<B>, took: Duration, _: &tracing::Span) {
    let status = response.status();
    if status == StatusCode::SWITCHING_PROTOCOLS {
        return;
    }
    let code = response.extensions().get::<ErrorCode>().map(tracing::field::display);
    let duration_ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
    // What the answer looks like on the wire: a proxy that rewrites it (a
    // compression, a cache) shows in these, while the status stays 200.
    let headers = response.headers();
    let bytes = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let content_encoding = headers
        .get(axum::http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok());
    let not_modified = status == StatusCode::NOT_MODIFIED;
    if status.is_server_error() {
        tracing::warn!(event = "request.answered", status = status.as_u16(), duration_ms, code, bytes, not_modified, content_encoding, "a request was answered");
    } else {
        tracing::info!(event = "request.answered", status = status.as_u16(), duration_ms, code, bytes, not_modified, content_encoding, "a request was answered");
    }
}
