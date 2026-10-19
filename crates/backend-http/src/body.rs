//! Request bodies read whole (`web-api.md` § Request bodies): a body over
//! its limit is refused with 413 `too_large` before more than that is read,
//! and before any of it when its declared length is over; a JSON body is at
//! most 1 MiB and must match its request type exactly; a page's WebSocket
//! message is at most 1 MiB too.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, FailedToBufferBody};
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::http::header::CONTENT_LENGTH;
use demi_web_api_protocol::MAX_PAGE_MESSAGE_BYTES;
use demi_web_api_protocol::error::{ErrorCode, problems};
use garde::Validate;
use serde::de::DeserializeOwned;
use tracing::Instrument as _;

use super::error::ApiError;

/// The most of a JSON body the backend reads. The router's
/// `DefaultBodyLimit` enforces it on the bytes; `JsonBody` refuses a larger
/// declared length before reading any.
pub(super) const JSON_BODY_LIMIT: usize = 1024 * 1024;

/// Upgrades to a page's WebSocket, which takes no message larger than
/// [`MAX_PAGE_MESSAGE_BYTES`]; a larger one fails the socket. `serve` runs
/// in the upgrade request's span, so the socket's whole life logs under
/// the request ID of its upgrade (`backend.md` § Request IDs and the server
/// log).
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
    Bytes::from_request(request, state)
        .await
        .map_err(|rejection| match rejection {
            BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => {
                too_large()
            }
            other => ApiError::invalid_body(other.body_text()),
        })
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
