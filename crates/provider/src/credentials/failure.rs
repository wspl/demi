//! Why a subscription account's credentials could not be used, as every
//! family reports it: to a run as its error, to the entry's status, to a
//! quota probe and to a catalog read. No message holds a token.

use demi_core::AuthState;

use crate::{
    CatalogError, ErrorCode, ProviderFailure,
    credentials::{AccountError, RenewError},
    quota::QuotaError,
};

/// Why a family's account could not be used, such as `Codex`'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthFailure {
    /// How messages name the family, such as `Codex` or `Grok`.
    pub family: &'static str,
    pub reason: AuthReason,
}

/// The reason of an [`AuthFailure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthReason {
    /// The provider stands for no account, as one built to log in, or its
    /// account has no secret document.
    Missing,
    /// The account's secret document is corrupt; the text says where.
    Invalid(String),
    /// The pool could not be read or written.
    Store(String),
    /// The vendor refused the refresh and nobody else stored new tokens; the
    /// text is the family's message.
    Refresh(String),
}

impl AuthFailure {
    pub fn new(family: &'static str, reason: AuthReason) -> Self {
        Self { family, reason }
    }

    /// The failure of an account whose secret could not be read.
    pub fn of_account(family: &'static str, error: AccountError) -> Self {
        let reason = match error {
            AccountError::Missing => AuthReason::Missing,
            AccountError::Invalid(error) => {
                AuthReason::Invalid(format!("its secret document is {error}"))
            }
            AccountError::Store(error) => AuthReason::Store(error.to_string()),
        };
        Self::new(family, reason)
    }

    /// The failure of a renewal whose refresh fails with the family's
    /// message.
    pub fn of_renewal(family: &'static str, error: RenewError<String>) -> Self {
        match error {
            RenewError::Account(error) => Self::of_account(family, error),
            RenewError::Refresh(message) => Self::new(family, AuthReason::Refresh(message)),
        }
    }

    pub fn message(&self) -> String {
        let family = self.family;
        match &self.reason {
            AuthReason::Missing => format!("No {family} account is signed in"),
            AuthReason::Invalid(detail) => format!("The {family} account cannot be read: {detail}"),
            AuthReason::Store(detail) => {
                format!("The {family} account could not be loaded: {detail}")
            }
            AuthReason::Refresh(message) => message.clone(),
        }
    }

    /// The run's failure: an authentication code, and none for a store that
    /// failed, which signing in again does not fix.
    pub fn failure(&self) -> ProviderFailure {
        let code = match self.reason {
            AuthReason::Missing => Some(ErrorCode::AuthMissing),
            AuthReason::Invalid(_) => Some(ErrorCode::AuthInvalid),
            AuthReason::Store(_) => None,
            AuthReason::Refresh(_) => Some(ErrorCode::AuthRefreshFailed),
        };
        ProviderFailure {
            message: self.message(),
            code,
            diagnostics: None,
            retry_after: None,
        }
    }

    /// The entry's status: unauthenticated without an account, an error
    /// otherwise.
    pub fn state(&self) -> AuthState {
        match self.reason {
            AuthReason::Missing => AuthState::Unauthenticated {
                message: Some(self.message()),
            },
            AuthReason::Invalid(_) | AuthReason::Store(_) | AuthReason::Refresh(_) => {
                AuthState::Error {
                    message: self.message(),
                }
            }
        }
    }

    /// A probe's failure: unauthenticated, or unavailable when the store
    /// failed.
    pub fn quota_error(&self) -> QuotaError {
        match self.reason {
            AuthReason::Store(_) => QuotaError::Unavailable(self.message()),
            AuthReason::Missing | AuthReason::Invalid(_) | AuthReason::Refresh(_) => {
                QuotaError::Unauthenticated(self.message())
            }
        }
    }

    /// A catalog read's failure, by the same rule as a probe's.
    pub fn catalog_error(&self) -> CatalogError {
        match self.reason {
            AuthReason::Store(_) => CatalogError::Unavailable(self.message()),
            AuthReason::Missing | AuthReason::Invalid(_) | AuthReason::Refresh(_) => {
                CatalogError::Unauthenticated(self.message())
            }
        }
    }
}
