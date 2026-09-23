//! The account operations of a subscription family (`providers.md` §
//! Credential vault): listing, selecting, logging in, adding and removing the
//! accounts of the provider's entry. The metadata they return is `core`'s,
//! because the browser receives it.

use demi_core::{AccountInfo, LoginPending};
use futures_util::future::BoxFuture;

use crate::Secret;

/// The accounts of one subscription entry.
pub trait SubscriptionAccounts: Send + Sync {
    fn capability(&self) -> AccountsCapability;

    fn list(&self) -> BoxFuture<'_, Result<Vec<AccountInfo>, AccountsError>>;

    /// The account the entry infers with, when it has one.
    fn active(&self) -> BoxFuture<'_, Result<Option<String>, AccountsError>>;

    fn set_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), AccountsError>>;

    /// Runs the family's device login and stores the new account, returning
    /// its id. `pending` receives what the user must do, once. Dropping the
    /// future cancels the login, even while it waits between polls.
    fn login<'a>(
        &'a self,
        pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> BoxFuture<'a, Result<String, LoginError>>;

    /// Stores an account from material the product supplies.
    fn add(&self, input: AddAccount) -> BoxFuture<'_, Result<AccountInfo, AccountsError>>;

    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), AccountsError>>;
}

/// Which ways a family offers to add an account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccountsCapability {
    /// A device login.
    pub login: bool,
    /// Adding supplied material, such as a Claude Code setup token.
    pub add: bool,
}

/// Material the product supplies for a new account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddAccount {
    /// A token that `claude setup-token` printed.
    SetupToken(Secret),
}

/// Why an account operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountsError {
    #[error("no account {0}")]
    NotFound(String),
    /// The family does not add accounts this way.
    #[error("this provider does not add accounts this way")]
    Unsupported,
    /// The supplied material is not an account of the family. The message
    /// never contains the material.
    #[error("{0}")]
    Invalid(String),
    /// The account store failed.
    #[error("{0}")]
    Store(String),
}

/// Why a login ended without an account.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoginError {
    /// The family has no device login.
    #[error("this provider has no device login")]
    Unsupported,
    /// The vendor does not offer the login, such as Codex answering 404.
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
}
