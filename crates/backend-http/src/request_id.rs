//! Request IDs (`backend.md` § Request IDs and the server log): the edge
//! names every request as it arrives, a WebSocket's upgrade included, and
//! never takes a name from the client. The ID is a field of the request's
//! `tracing` span, every response carries it in `X-Request-Id`, and every
//! JSON error carries it as `requestId`.

use std::sync::LazyLock;

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use data_encoding::{Encoding, Specification};
use demi_shared_types::RequestId;

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
