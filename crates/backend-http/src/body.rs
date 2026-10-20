//! Request bodies read whole.
//!
//! - A body over its limit is refused with 413 `too_large` before more than
//!   that is read, and before any of it when its declared length is over.
//! - A JSON body is at most 1 MiB and must match its request type exactly.
//! - A page's WebSocket message is at most 1 MiB too.
//! - A body of which nothing arrives for a minute answers 408
//!   `transfer_stalled`, and the request ends there.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::{BytesRejection, FailedToBufferBody};
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::http::header::CONTENT_LENGTH;
use demi_web_api_protocol::MAX_PAGE_MESSAGE_BYTES;
use demi_web_api_protocol::error::{ErrorCode, problems};
use futures_util::StreamExt as _;
use garde::Validate;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use super::error::ApiError;
use super::transfer::TRANSFER_IDLE;

/// The most of a JSON body the backend reads. The router's
/// `DefaultBodyLimit` enforces it on the bytes; `JsonBody` refuses a larger
/// declared length before reading any.
pub(super) const JSON_BODY_LIMIT: usize = 1024 * 1024;

/// Upgrades to a page's WebSocket, which takes no message larger than
/// [`MAX_PAGE_MESSAGE_BYTES`]; a larger one fails the socket. `serve` runs
/// in the upgrade request's span, so the socket's whole life logs under the
/// request ID of its upgrade.
pub(super) fn page_socket<F, Fut>(upgrade: WebSocketUpgrade, serve: F) -> Response
where
    F: FnOnce(WebSocket) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let span = tracing::Span::current();
    upgrade
        .max_message_size(MAX_PAGE_MESSAGE_BYTES)
        .max_frame_size(MAX_PAGE_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve(socket).instrument(span))
}

/// A JSON body decoded into its request type and validated. The content type
/// is not consulted; a body that is empty, malformed or does not match the
/// type answers 400 `invalid_body` naming the field and the reason.
pub(super) struct JsonBody<T>(pub(super) T);

/// A JSON body a route may also receive empty, which then reads as `T`'s
/// default; any other body follows [`JsonBody`]'s rules.
pub(super) struct OptionalJsonBody<T>(pub(super) T);

/// A body that is any JSON document, an empty one reading as `{}`, for a
/// route whose callee checks it against a schema of its own.
pub(super) struct JsonValueBody(pub(super) serde_json::Value);

impl<S: Send + Sync> FromRequest<S> for JsonValueBody {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = read(request, state).await?;
        if bytes.is_empty() {
            return Ok(Self(serde_json::Value::Object(serde_json::Map::new())));
        }
        let Json(value) = Json::<serde_json::Value>::from_bytes(&bytes)
            .map_err(|rejection| ApiError::invalid_body(rejection.body_text()))?;
        Ok(Self(value))
    }
}

fn too_large() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::TooLarge,
        format!("The request body is over its {JSON_BODY_LIMIT}-byte limit"),
    )
    .with_param("limitBytes", JSON_BODY_LIMIT)
}

impl<S, T> FromRequest<S> for JsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate<Context = ()>,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = read(request, state).await?;
        Ok(Self(decode(&bytes)?))
    }
}

impl<S, T> FromRequest<S> for OptionalJsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate<Context = ()> + Default,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = read(request, state).await?;
        if bytes.is_empty() {
            return Ok(Self(T::default()));
        }
        Ok(Self(decode(&bytes)?))
    }
}

/// The JSON body's bytes, within its limit.
pub(super) async fn read<S: Send + Sync>(request: Request, state: &S) -> Result<Bytes, ApiError> {
    read_within(request, state, JSON_BODY_LIMIT, too_large).await
}

/// The body's bytes, at most `limit`, which the route's `DefaultBodyLimit`
/// sets for the bytes as they arrive. A declared length over `limit` is
/// refused before a byte is read, and a body without one is counted as it
/// arrives; either way the refusal is `too_large`'s.
///
/// A body that stops arriving answers 408 `transfer_stalled` once nothing of
/// it came for [`TRANSFER_IDLE`]; without the bound, a client or a proxy that
/// stops sending mid-body holds the request, its task and the bytes read so
/// far until the connection dies, which may be hours. The bound is on the
/// wait between two reads, as nginx's `client_body_timeout` is, not on the
/// whole body: a JSON body is small, but a 25 MiB attachment on a slow link
/// takes minutes and still arrives as long as it moves.
pub(super) async fn read_within<S: Send + Sync>(
    request: Request,
    state: &S,
    limit: usize,
    too_large: fn() -> ApiError,
) -> Result<Bytes, ApiError> {
    let declared = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limit as u64) {
        return Err(too_large());
    }
    let stalled = Arc::new(AtomicBool::new(false));
    let request = request.map(|body| stall_bounded(body, stalled.clone()));
    Bytes::from_request(request, state)
        .await
        .map_err(|rejection| match rejection {
            BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => {
                too_large()
            }
            _ if stalled.load(Ordering::Relaxed) => ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                ErrorCode::TransferStalled,
                "Nothing of the request body arrived for a minute",
            ),
            other => ApiError::invalid_body(other.body_text()),
        })
}

/// `body`, failing once nothing of it arrives for [`TRANSFER_IDLE`], which
/// `stalled` then says. The flag tells the stall from the other failures of
/// the body, which axum's rejection reports all alike. The route's body
/// limit still applies, since axum wraps this body in it as it reads.
fn stall_bounded(body: Body, stalled: Arc<AtomicBool>) -> Body {
    let chunks = futures_util::stream::unfold(
        Some((body.into_data_stream(), stalled)),
        |state| async move {
            let (mut chunks, stalled) = state?;
            match tokio::time::timeout(TRANSFER_IDLE, chunks.next()).await {
                Ok(Some(chunk)) => Some((chunk, Some((chunks, stalled)))),
                Ok(None) => None,
                Err(_) => {
                    stalled.store(true, Ordering::Relaxed);
                    let error = std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "nothing of the body arrived for a minute",
                    );
                    Some((Err(axum::Error::new(error)), None))
                }
            }
        },
    );
    Body::from_stream(chunks)
}

/// `bytes` as `T`, checked by its rules.
fn decode<T: DeserializeOwned + Validate<Context = ()>>(bytes: &[u8]) -> Result<T, ApiError> {
    decode_worded(bytes, problems)
}

/// A body whose broken rules are refused in words for the user, which the
/// page shows as they are, such as a port forward's or a credential's.
pub(super) trait Worded {
    /// The words for what `report` found broken.
    fn refusal(report: &garde::Report) -> String;
}

/// A [`Worded`] body, read as a JSON body is.
pub(super) struct WordedBody<T>(pub T);

impl<S, T> axum::extract::FromRequest<S> for WordedBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate<Context = ()> + Worded,
{
    type Rejection = ApiError;

    async fn from_request(request: axum::extract::Request, state: &S) -> Result<Self, ApiError> {
        let bytes = read(request, state).await?;
        Ok(Self(decode_worded(&bytes, T::refusal)?))
    }
}

/// `bytes` as `T`, checked by its rules; a broken rule is refused in the
/// words `words` gives it.
fn decode_worded<T: DeserializeOwned + Validate<Context = ()>>(
    bytes: &[u8],
    words: fn(&garde::Report) -> String,
) -> Result<T, ApiError> {
    let Json(value) = Json::<T>::from_bytes(bytes)
        .map_err(|rejection| ApiError::invalid_body(rejection.body_text()))?;
    value
        .validate()
        .map_err(|report| ApiError::invalid_body(words(&report)))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::response::IntoResponse as _;
    use futures_util::stream;
    use tokio::time::Instant;

    use super::*;

    /// A request whose body's `chunks` arrive `apart` from each other, after
    /// which it ends, or, when it `stalls`, sends nothing more, ever.
    fn arriving(chunks: &[&'static str], apart: Duration, stalls: bool) -> Request {
        let sent = stream::iter(chunks.to_vec()).then(move |chunk| async move {
            tokio::time::sleep(apart).await;
            Ok::<_, std::io::Error>(Bytes::from_static(chunk.as_bytes()))
        });
        let body = if stalls {
            Body::from_stream(sent.chain(stream::pending()))
        } else {
            Body::from_stream(sent)
        };
        Request::post("/api/conversations/c/read")
            .header(CONTENT_LENGTH, 64)
            .body(body)
            .unwrap()
    }

    // Paused time: the minute passes as soon as nothing else can run, so
    // each scenario takes milliseconds.
    #[tokio::test(start_paused = true)]
    async fn a_body_that_stops_arriving_answers_408_within_a_minute_of_its_last_byte() {
        let started = Instant::now();
        let read = tokio::time::timeout(
            TRANSFER_IDLE * 3,
            JsonValueBody::from_request(arriving(&["{\"rev", "ision\":"], Duration::ZERO, true), &()),
        )
        .await
        .expect("the read ends within the bound");
        let Err(error) = read else {
            panic!("a stalled body reads as refused");
        };
        assert_eq!(started.elapsed(), TRANSFER_IDLE);
        assert_eq!(error.code(), ErrorCode::TransferStalled);
        assert_eq!(error.into_response().status(), StatusCode::REQUEST_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn a_body_that_keeps_arriving_is_read_whole_however_long_it_takes() {
        let apart = TRANSFER_IDLE - Duration::from_secs(1);
        let request = arriving(&["{\"rev", "ision\"", ":7}"], apart, false);
        let read = JsonValueBody::from_request(request, &()).await;
        let Ok(JsonValueBody(value)) = read else {
            panic!("a slow body is read");
        };
        assert_eq!(value, serde_json::json!({ "revision": 7 }));
    }
}
