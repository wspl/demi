//! Request IDs (`backend.md` § Request IDs and the server log): the edge
//! names every request as it arrives, a WebSocket's upgrade included, and
//! never takes a name from the client. The ID is a field of the request's
//! `tracing` span, every response carries it in `X-Request-Id`, and every
//! JSON error carries it as `requestId`. Every request logs one line as it
//! is answered, or as its client abandons it, so that an ID a user copies
//! always finds its request.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use data_encoding::{Encoding, Specification};
use demi_shared_types::RequestId;
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

/// The span every event of `request` is logged in: its ID, method and path,
/// never its query, which can carry a file's path or a token, and
/// Cloudflare's `cf-ray` when a proxy passes it. The session gate records
/// the caller as `user_id`.
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
        path = request.uri().path(),
        cf_ray = ray,
        user_id = tracing::field::Empty,
    )
}

/// How long an abandoned request runs before its line is a warning.
const SLOW_ABANDONED: Duration = Duration::from_secs(10);

/// Logs *a request was abandoned* for a request whose client went away
/// before its answer, a closed tab or a timed-out fetch: hyper then drops
/// the answer's future, and with it this middleware's guard, which no
/// answer released. Runs inside the request's span, which it logs in, so
/// the line carries the span's ID, method, path, user and `cf-ray`. The
/// work the request started goes on or stops as that work's own rule says.
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
                tracing::warn!(duration_ms, "a request was abandoned");
            } else {
                tracing::info!(duration_ms, "a request was abandoned");
            }
        });
    }
}

/// The line a request logs as it is answered, in its span: the status, how
/// long it took and the `code` of a JSON error, at `info`, and at `warn` for
/// a 5xx answer. An upgrade's answer logs nothing: its socket's end line is
/// its line.
pub(crate) fn answered<B>(response: &axum::http::Response<B>, took: Duration, _: &tracing::Span) {
    let status = response.status();
    if status == StatusCode::SWITCHING_PROTOCOLS {
        return;
    }
    let code = response.extensions().get::<ErrorCode>().map(tracing::field::display);
    let duration_ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
    if status.is_server_error() {
        tracing::warn!(status = status.as_u16(), duration_ms, code, "a request was answered");
    } else {
        tracing::info!(status = status.as_u16(), duration_ms, code, "a request was answered");
    }
}
