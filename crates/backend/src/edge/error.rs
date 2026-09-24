//! The edge's error answer: an HTTP status with a JSON `ErrorBody`, and the
//! mapping of each domain error to it.

use axum::Json;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_web_api::error::{ErrorBody, ErrorCode};

use demi_shell::HostError;

use crate::auth::email_change::EmailChangeError;
use crate::auth::passwords::HashError;
use crate::conversation::host_access::{HostAccessError, host_error_code};
use crate::conversation::stream::StreamError;
use crate::llm::assembly::AssemblyError;
use crate::runner::files::{TextError, TextRefusal};
use crate::shard::ShardUnavailable;
use crate::storage::StorageError;
use crate::vault::accounts::AccountRefusal;
use crate::vault::logins::LoginRefusal;

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

    pub(crate) fn invalid_query(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery, message)
    }

    pub(crate) fn backend_closing() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::BackendClosing,
            "The backend is shutting down",
        )
    }

    pub(crate) fn no_route(method: &Method, path: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, format!("No route for {method} {path}"))
    }

    pub(crate) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, message)
    }

    pub(crate) fn provider_not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::ProviderNotFound, "No such provider")
    }

    pub(crate) fn provider_busy() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::ProviderBusy,
            "Another change of this provider is still running",
        )
    }

    pub(crate) fn account_not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::AccountNotFound, "No such account")
    }

    #[cfg(test)]
    pub(crate) fn code(&self) -> ErrorCode {
        self.body.code
    }

    pub(crate) fn device_offline() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::DeviceOffline, "The device's runner is not connected")
    }

    /// A Host's failure of a file operation (`host_error_code`).
    pub(crate) fn host_operation(error: HostError) -> Self {
        let (code, status) = host_error_code(&error);
        Self::new(status_of(status), code, error.message)
    }

    /// A Host's failure of a working-tree operation: the runner's git codes,
    /// and otherwise as a file operation's.
    pub(crate) fn working_tree(error: HostError) -> Self {
        let message = error.message.clone();
        match error.code() {
            Some("timeout") => Self::new(StatusCode::GATEWAY_TIMEOUT, ErrorCode::ChangesTimeout, message),
            Some("not_repository") => Self::new(StatusCode::CONFLICT, ErrorCode::NotRepository, message),
            Some("too_large") => Self::new(StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::FileTooLarge, message),
            Some("ENOENT" | "EACCES" | "EPERM") | None => Self::host_operation(error),
            Some(_) => Self::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::ChangesFailed, message),
        }
    }

    /// A failure a request could not cause, in words: logged, and answered
    /// 500.
    pub(crate) fn internal_message(message: impl Into<String>) -> Self {
        let message = message.into();
        tracing::error!(message, "a request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError, message)
    }

    /// A failure a request could not cause: logged with its causes, and
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

impl From<ShardUnavailable> for ApiError {
    fn from(error: ShardUnavailable) -> Self {
        match error {
            ShardUnavailable::Closing => Self::backend_closing(),
            ShardUnavailable::Failed => Self::internal(&error),
        }
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

impl From<AssemblyError> for ApiError {
    fn from(error: AssemblyError) -> Self {
        Self::internal(&error)
    }
}

impl From<AccountRefusal> for ApiError {
    fn from(refusal: AccountRefusal) -> Self {
        let message = refusal.to_string();
        match refusal {
            AccountRefusal::Exists => Self::new(StatusCode::CONFLICT, ErrorCode::ProviderExists, message),
            AccountRefusal::Unsupported(_) => Self::new(StatusCode::BAD_REQUEST, ErrorCode::AccountsUnsupported, message),
            AccountRefusal::NotFound => Self::account_not_found(),
            AccountRefusal::Active => Self::new(StatusCode::CONFLICT, ErrorCode::ActiveAccount, message),
            AccountRefusal::TokenImportFailed => Self::new(StatusCode::BAD_REQUEST, ErrorCode::TokenImportFailed, message),
            AccountRefusal::Store(_) | AccountRefusal::Assembly(_) => Self::internal(&refusal),
        }
    }
}

impl From<LoginRefusal> for ApiError {
    fn from(refusal: LoginRefusal) -> Self {
        let message = refusal.to_string();
        match refusal {
            LoginRefusal::NoLoginFlow(_) => Self::new(StatusCode::BAD_REQUEST, ErrorCode::NoLoginFlow, message),
            LoginRefusal::Exists(_) => Self::new(StatusCode::CONFLICT, ErrorCode::ProviderExists, message),
            LoginRefusal::Busy => Self::provider_busy(),
            LoginRefusal::Assembly(AssemblyError::UnknownFamily(_)) => {
                Self::new(StatusCode::BAD_REQUEST, ErrorCode::UnknownProviderType, message)
            }
            LoginRefusal::Assembly(_) => Self::internal(&refusal),
        }
    }
}

impl From<EmailChangeError> for ApiError {
    fn from(error: EmailChangeError) -> Self {
        Self::internal(&error)
    }
}

impl From<HostAccessError> for ApiError {
    fn from(error: HostAccessError) -> Self {
        match error {
            HostAccessError::Host(error) => Self::host_operation(error),
            HostAccessError::Cancelled | HostAccessError::Storage(_) => Self::internal(&error),
            error => {
                let (code, status) = error.code();
                Self::new(status_of(status), code, error.to_string())
            }
        }
    }
}

impl From<StreamError> for ApiError {
    fn from(error: StreamError) -> Self {
        match error {
            StreamError::Access(error) => error.into(),
            StreamError::Failed(_) => {
                let (code, status) = error.code();
                Self::new(status_of(status), code, error.to_string())
            }
        }
    }
}

/// An HTTP status a refusal names.
fn status_of(status: u16) -> StatusCode {
    StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

impl From<TextError> for ApiError {
    fn from(error: TextError) -> Self {
        match error {
            TextError::Host(error) => Self::host_operation(error),
            TextError::Refused(refusal) => refusal.into(),
        }
    }
}

impl From<TextRefusal> for ApiError {
    fn from(refusal: TextRefusal) -> Self {
        let message = refusal.to_string();
        match refusal {
            TextRefusal::TooLarge => Self::new(StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::FileTooLarge, message),
            TextRefusal::NotText => Self::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, ErrorCode::NotText, message),
        }
    }
}
