//! Request bodies read whole (`web-api.md` § Request bodies): a JSON body is
//! at most 1 MiB, refused with 413 `too_large` before more than that is read,
//! and must match its request type exactly.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, FailedToBufferBody};
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::http::header::CONTENT_LENGTH;
use demi_web_api::error::ErrorCode;
use garde::Validate;
use serde::de::DeserializeOwned;

use super::error::ApiError;

/// The most of a JSON body the backend reads. The router's
/// `DefaultBodyLimit` enforces it on the bytes; `JsonBody` refuses a larger
/// declared length before reading any.
pub(super) const JSON_BODY_LIMIT: usize = 1024 * 1024;

/// A JSON body decoded into its request type and validated. The content type
/// is not consulted; a body that is empty, malformed or does not match the
/// type answers 400 `invalid_body` naming the field and the reason.
pub(super) struct JsonBody<T>(pub(super) T);

/// A JSON body a route may also receive empty, which then reads as `T`'s
/// default; any other body follows [`JsonBody`]'s rules.
pub(super) struct OptionalJsonBody<T>(pub(super) T);

fn too_large() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::TooLarge,
        format!("The request body is over its {JSON_BODY_LIMIT}-byte limit"),
    )
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

/// The body's bytes, within the limit.
async fn read<S: Send + Sync>(request: Request, state: &S) -> Result<Bytes, ApiError> {
        let declared = request
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|length| length.to_str().ok())
            .and_then(|length| length.parse::<u64>().ok());
        if declared.is_some_and(|length| length > JSON_BODY_LIMIT as u64) {
            return Err(too_large());
        }
        Bytes::from_request(request, state).await.map_err(|rejection| match rejection {
            BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => too_large(),
            other => ApiError::invalid_body(other.body_text()),
        })
}

/// `bytes` as `T`, checked by its rules.
fn decode<T: DeserializeOwned + Validate<Context = ()>>(bytes: &[u8]) -> Result<T, ApiError> {
    let Json(value) = Json::<T>::from_bytes(bytes).map_err(|rejection| ApiError::invalid_body(rejection.body_text()))?;
    value.validate().map_err(|report| {
        let problems: Vec<String> = report.iter().map(|(path, error)| format!("{path}: {error}")).collect();
        ApiError::invalid_body(problems.join("; "))
    })?;
    Ok(value)
}
