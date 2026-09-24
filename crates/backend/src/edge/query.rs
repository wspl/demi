//! Query parameters decoded into their types (`web-api.md` § Query
//! parameters): a value outside its type answers 400 `invalid_query`.

use axum::extract::{FromRequestParts, Query};
use axum::http::StatusCode;
use axum::http::request::Parts;
use demi_web_api::error::ErrorCode;
use serde::de::DeserializeOwned;

use super::error::ApiError;

/// The request's query as `T`.
pub(super) struct QueryParams<T>(pub(super) T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for QueryParams<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| {
                ApiError::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery, rejection.body_text())
            })?;
        Ok(Self(value))
    }
}
