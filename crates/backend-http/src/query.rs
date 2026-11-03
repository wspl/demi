//! Query parameters decoded into their types: a value outside its type answers
//! 400 `invalid_query`.

use axum::extract::{FromRequestParts, Query};
use axum::http::StatusCode;
use axum::http::request::Parts;
use demi_web_api_protocol::error::ErrorCode;
use garde::Validate;
use serde::de::DeserializeOwned;

use super::error::ApiError;

/// The request's query as `T`.
pub(super) struct QueryParams<T>(pub(super) T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for QueryParams<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let Query(value) =
            Query::<T>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        ErrorCode::InvalidQuery,
                        rejection.body_text(),
                    )
                })?;
        Ok(Self(value))
    }
}

/// The request's query as `T`, checked by its rules too: a value its type
/// takes but its rules refuse, such as a limit out of range, answers 400
/// `invalid_query` as well.
pub(super) struct CheckedQuery<T>(pub(super) T);

impl<S: Send + Sync, T: DeserializeOwned + Validate<Context = ()>> FromRequestParts<S> for CheckedQuery<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let QueryParams(value) = QueryParams::<T>::from_request_parts(parts, state).await?;
        value
            .validate()
            .map_err(|report| ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery, report.to_string()))?;
        Ok(Self(value))
    }
}
