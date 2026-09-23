//! The body of every JSON error, and the one list of the codes it carries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A JSON error answer: `{ code, message }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    /// Says what happened in words; for a refused body it names the field and
    /// the reason.
    pub message: String,
}

/// Every error code the browser can see. A situation has one code on every
/// route (`web-api.md` § Resource index); the HTTP status belongs to the
/// route that answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request carries no live session.
    Unauthenticated,
    /// No route answers the method and path, or the path names nothing the
    /// caller holds, such as a blob outside the caller's namespace.
    NotFound,
    /// A JSON body does not match its request type.
    InvalidBody,
    /// A body the backend reads whole is over its limit.
    TooLarge,
    /// The backend failed in a way the request could not cause.
    InternalError,
    /// The backend is shutting down and starts no new work.
    BackendClosing,
    /// Setup has created the master account already.
    AlreadySetUp,
    /// The email address and password, or the current password, do not match.
    InvalidCredentials,
    /// Too many attempts for now: five failed logins for one address within
    /// a minute of each other, or a new verification code within a minute of
    /// the last one.
    TooManyAttempts,
    /// Another account, or the caller's own, has the email address.
    EmailTaken,
    /// The verification code is wrong, expired or used up, or its challenge
    /// no longer holds.
    InvalidCode,
    /// The backend has no account mail sender to deliver a verification code.
    MailUnavailable,
    /// The verification mail could not be delivered.
    MailFailed,
}

serde_plain::derive_display_from_serialize!(ErrorCode);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_body_travels_as_code_and_message() {
        let body = ErrorBody {
            code: ErrorCode::AlreadySetUp,
            message: "This instance has its master account".to_owned(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "code": "already_set_up",
                "message": "This instance has its master account",
            })
        );
        assert_eq!(serde_json::from_value::<ErrorBody>(json).unwrap(), body);
    }
}
