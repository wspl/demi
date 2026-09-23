//! The edge's error answer: an HTTP status with a JSON `ErrorBody`, and the
//! mapping of each domain error to it.

use axum::Json;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_web_api::error::{ErrorBody, ErrorCode};

use crate::auth::email_change::EmailChangeError;
use crate::auth::passwords::HashError;
use crate::storage::StorageError;

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    body: ErrorBody,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorBody {
                code,
                message: message.into(),
            },
        }
    }

    pub(crate) fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated, "Sign in first")
    }

    pub(crate) fn invalid_body(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidBody, message)
    }

    pub(crate) fn no_route(method: &Method, path: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, format!("No route for {method} {path}"))
    }

    /// A failure the request could not cause: logged with its causes, and
    /// answered 500.
    fn internal(error: &(dyn std::error::Error + 'static)) -> Self {
        tracing::error!(error, "a request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError, error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<StorageError> for ApiError {
    fn from(error: StorageError) -> Self {
        Self::internal(&error)
    }
}

impl From<HashError> for ApiError {
    fn from(error: HashError) -> Self {
        Self::internal(&error)
    }
}

impl From<EmailChangeError> for ApiError {
    fn from(error: EmailChangeError) -> Self {
        Self::internal(&error)
    }
}
