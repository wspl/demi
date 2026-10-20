//! The edge's error answer: an HTTP status with a JSON `ErrorBody`, and the
//! mapping of each domain error to it.

use demi_web_api_protocol::workspaces::WorkspaceDto;
use axum::Json;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use std::collections::BTreeMap;

use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
use demi_web_api_protocol::ids::LoginId;
use demi_web_api_protocol::passkeys::PasskeyId;

use crate::request_id;

use demi_backend_accounts::email_change::EmailChangeError;
use demi_backend_accounts::passkeys::PasskeyError;
use demi_backend_accounts::passwords::HashError;
use demi_backend_blobs::ObjectError;
use demi_backend_cloud::machine::CloudError;
use demi_backend_database::StorageError;
use demi_backend_providers::llm::assembly::AssemblyError;
use demi_backend_providers::vault::accounts::AccountRefusal;
use demi_backend_providers::vault::logins::LoginRefusal;
use demi_backend_runners::files::{TextError, TextRefusal};
use demi_host_interface::HostError;
use demi_shared_types::ReadAt;

use demi_backend_host_access::access::{HostAccessError, host_error_code};
use demi_backend_host_access::stream::StreamError;
use demi_backend_user_shard::shard::ShardUnavailable;

/// The header that carries the Host's number a read answer was read at.
pub(crate) const READ_AT: HeaderName = HeaderName::from_static("x-demi-read-at");

/// `<epoch>.<number>`, as [`READ_AT`] carries it; none for an epoch a
/// header cannot carry, which no runner sends.
pub(crate) fn read_at_value(read_at: &ReadAt) -> Option<HeaderValue> {
    HeaderValue::from_str(&read_at.to_string()).ok()
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: ErrorCode,
    message: String,
    /// The values the page's text for the code needs.
    params: BTreeMap<String, serde_json::Value>,
    /// A plugin's own word for its refusal.
    reason: Option<String>,
    /// The passkey a refused sign-in names.
    credential_id: Option<PasskeyId>,
    /// The starter's own login that holds the entry a login start was
    /// refused for.
    login_id: Option<LoginId>,
    /// The workspace a `workspace_exists` answer names.
    workspace: Option<WorkspaceDto>,
    /// The `Retry-After` the answer carries, in whole seconds.
    retry_after: Option<u64>,
    /// The ETag of the file a refused read is an answer of, such as one
    /// that is binary.
    etag: Option<String>,
    /// The Host's number before the read the failure answers, which the
    /// answer carries as `X-Demi-Read-At`.
    read_at: Option<ReadAt>,
}

impl ApiError {
    /// The backend's own answer, which carries its `{ code, message }` and
    /// the request's ID.
    ///
    /// It is never 502 or 504, even for a vendor or a Host that failed or did
    /// not answer in time: a CDN in front of the backend, such as Cloudflare
    /// (`crates/backend/src/config/mod.rs`), replaces those answers with a
    /// page of its own, and the web app reads a 502, 503 or 504 without the
    /// backend's body as the backend being away and sends the request again
    /// until it is back. Such a failure answers 503, which the CDN passes
    /// through with the body.
    pub(crate) fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            params: BTreeMap::new(),
            reason: None,
            credential_id: None,
            login_id: None,
            workspace: None,
            retry_after: None,
            etag: None,
            read_at: None,
        }
    }

    /// The error with the value `name` that the page's text for its code
    /// is filled with.
    pub(crate) fn with_param(mut self, name: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }

    /// The error as the answer of a read the Host took at `read_at`.
    pub(crate) fn with_read_at(mut self, read_at: ReadAt) -> Self {
        self.read_at = Some(read_at);
        self
    }

    /// The error as the answer of the file at version `etag`.
    pub(crate) fn with_etag(mut self, etag: String) -> Self {
        self.etag = Some(etag);
        self
    }

    /// The error with how long, at least, the client waits before trying
    /// again, as `Retry-After` says it; rounded up to whole seconds.
    pub(crate) fn with_retry_after(mut self, wait: std::time::Duration) -> Self {
        self.retry_after = Some(wait.as_secs() + u64::from(wait.subsec_nanos() > 0));
        self
    }

    /// The error as the answer that names the passkey `id`.
    pub(crate) fn with_credential_id(mut self, id: PasskeyId) -> Self {
        self.credential_id = Some(id);
        self
    }

    /// The error as the answer that names the user's workspace `workspace`.
    pub(crate) fn with_workspace(mut self, workspace: WorkspaceDto) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// The error with a plugin's own word for its refusal.
    pub(crate) fn with_reason(mut self, reason: String) -> Self {
        self.reason = Some(reason);
        self
    }

    pub(crate) fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "Sign in first",
        )
    }

    /// A disabled account's sign-in, by any way it signs in.
    pub(crate) fn account_disabled() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ErrorCode::AccountDisabled,
            "This account is disabled",
        )
    }

    /// A request of an account still to choose its own password, which
    /// nothing but choosing it, signing out and the passkey routes answers.
    pub(crate) fn password_choice_required() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ErrorCode::PasswordChoiceRequired,
            "Choose your own password first",
        )
    }

    /// What the error says, as its answer's `message`.
    pub(crate) fn message(&self) -> &str {
        &self.message
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
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            format!("No route for {method} {path}"),
        )
    }

    pub(crate) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, message)
    }

    pub(crate) fn provider_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::ProviderNotFound,
            "No such provider",
        )
    }

    pub(crate) fn provider_busy() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::ProviderBusy,
            "Another change of this provider is still running",
        )
    }

    pub(crate) fn account_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::AccountNotFound,
            "No such account",
        )
    }

    #[cfg(test)]
    pub(crate) fn code(&self) -> ErrorCode {
        self.code
    }

    pub(crate) fn device_offline() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::DeviceOffline,
            "The device's runner is not connected",
        )
    }

    /// A Host's failure of a file operation (`host_error_code`).
    pub(crate) fn host_operation(error: HostError) -> Self {
        let (code, status) = host_error_code(&error);
        let answered = Self::new(status_of(status), code, error.message);
        match error.read_at {
            Some(read_at) => answered.with_read_at(read_at),
            None => answered,
        }
    }

    /// A Host's failure of a history read: a commit or a branch the repository
    /// does not have is 404 `not_found`, and otherwise as a working-tree
    /// operation's.
    pub(crate) fn history(error: HostError) -> Self {
        if error.code() != Some("ENOENT") {
            return Self::working_tree(error);
        }
        let answered = Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, error.message);
        match error.read_at {
            Some(read_at) => answered.with_read_at(read_at),
            None => answered,
        }
    }

    /// A Host's failure of a working-tree operation: the runner's git codes,
    /// and otherwise as a file operation's.
    pub(crate) fn working_tree(error: HostError) -> Self {
        let message = error.message.clone();
        let read_at = error.read_at.clone();
        let answered = match error.code() {
            Some("timeout") => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::ChangesTimeout,
                message,
            ),
            Some("not_repository") => {
                Self::new(StatusCode::CONFLICT, ErrorCode::NotRepository, message)
            }
            Some("ambiguous") => Self::new(StatusCode::CONFLICT, ErrorCode::Ambiguous, message),
            Some("too_large") => Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorCode::FileTooLarge,
                message,
            ),
            Some("ENOENT" | "EACCES" | "EPERM") | None => Self::host_operation(error),
            Some(_) => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::ChangesFailed,
                message,
            ),
        };
        match read_at {
            Some(read_at) => answered.with_read_at(read_at),
            None => answered,
        }
    }

    /// A conversation's database that could not be read: logged with its
    /// causes, and answered 500 `conversation_unreadable`, whose words the
    /// page chooses. The storage closing or its work not finishing is no
    /// fault of the database, and answers as any storage failure does.
    pub(crate) fn conversation_unreadable(error: StorageError) -> Self {
        match error {
            StorageError::Closed | StorageError::Interrupted(_) => error.into(),
            error => {
                tracing::error!(
                    error = &error as &dyn std::error::Error,
                    "a conversation's database could not be read"
                );
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::ConversationUnreadable,
                    error.to_string(),
                )
            }
        }
    }

    /// A failure a request could not cause, in words: logged, and answered
    /// 500.
    pub(crate) fn internal_message(message: impl Into<String>) -> Self {
        let message = message.into();
        tracing::error!(message, "a request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::InternalError,
            message,
        )
    }

    /// A failure a request could not cause: logged with its causes, and
    /// answered 500.
    fn internal(error: &(dyn std::error::Error + 'static)) -> Self {
        tracing::error!(error, "a request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::InternalError,
            error.to_string(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: self.code,
            message: self.message,
            params: self.params,
            reason: self.reason,
            credential_id: self.credential_id,
            login_id: self.login_id,
            workspace: self.workspace,
            request_id: request_id::current_or_new(),
        };
        let mut response = (self.status, Json(body)).into_response();
        // The request's answered line names the code.
        response.extensions_mut().insert(self.code);
        let headers = response.headers_mut();
        if let Some(seconds) = self.retry_after {
            headers.insert(axum::http::header::RETRY_AFTER, seconds.into());
        }
        if let Some(etag) = self.etag.and_then(|etag| HeaderValue::from_str(&etag).ok()) {
            headers.insert(axum::http::header::ETAG, etag);
        }
        if let Some(read_at) = self.read_at.as_ref().and_then(read_at_value) {
            headers.insert(READ_AT, read_at);
        }
        response
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

impl From<PasskeyError> for ApiError {
    fn from(error: PasskeyError) -> Self {
        Self::internal(&error)
    }
}

impl From<ObjectError> for ApiError {
    fn from(error: ObjectError) -> Self {
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
            AccountRefusal::Unsupported(_) => Self::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::AccountsUnsupported,
                message,
            ),
            AccountRefusal::NotFound => Self::account_not_found(),
            AccountRefusal::Order => Self::invalid_body(message),
            AccountRefusal::Assembly(_) | AccountRefusal::Forget(_) => Self::internal(&refusal),
        }
    }
}

impl From<LoginRefusal> for ApiError {
    fn from(refusal: LoginRefusal) -> Self {
        let message = refusal.to_string();
        match refusal {
            LoginRefusal::NoLoginFlow(_) => {
                Self::new(StatusCode::BAD_REQUEST, ErrorCode::NoLoginFlow, message)
            }
            LoginRefusal::Exists(_) => {
                Self::new(StatusCode::CONFLICT, ErrorCode::ProviderExists, message)
            }
            LoginRefusal::Busy(login) => Self {
                login_id: login,
                ..Self::provider_busy()
            },
            LoginRefusal::Assembly(AssemblyError::UnknownFamily(_)) => Self::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::UnknownProviderType,
                message,
            ),
            LoginRefusal::Assembly(_) => Self::internal(&refusal),
        }
    }
}

impl From<EmailChangeError> for ApiError {
    fn from(error: EmailChangeError) -> Self {
        Self::internal(&error)
    }
}

impl From<CloudError> for ApiError {
    fn from(error: CloudError) -> Self {
        let (code, status) = error.code();
        Self::new(status_of(status), code, error.to_string())
    }
}

impl From<HostAccessError> for ApiError {
    fn from(error: HostAccessError) -> Self {
        Self::from(&error)
    }
}

/// A refusal kept and answered more than once, as a kept search's.
impl From<&HostAccessError> for ApiError {
    fn from(error: &HostAccessError) -> Self {
        match error {
            HostAccessError::Host(error) => Self::host_operation(error.clone()),
            HostAccessError::Cancelled | HostAccessError::Storage(_) => Self::internal(error),
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
pub(crate) fn status_of(status: u16) -> StatusCode {
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
            TextRefusal::TooLarge => Self::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorCode::FileTooLarge,
                message,
            ),
            TextRefusal::NotText => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ErrorCode::NotText,
                message,
            ),
        }
    }
}
