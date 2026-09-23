//! The account operations of a subscription family (`providers.md` §
//! Credential vault): listing, selecting, logging in, adding and removing the
//! accounts of the provider's entry. One implementation, [`Accounts`], serves
//! every family over its credential pool; a family supplies only what differs,
//! its [`AccountKit`]: how it logs in or takes supplied material, and how an
//! account names itself.

use std::sync::Arc;

use demi_core::{AccountInfo, Clock, LoginPending};
use futures_util::future::BoxFuture;

use crate::{
    Secret,
    credentials::{AccountMeta, CredentialPool, PoolError, credential_id_for, find_by_identity},
};

/// The accounts of one subscription entry.
pub trait SubscriptionAccounts: Send + Sync {
    fn capability(&self) -> AccountsCapability;

    /// The entry's accounts, ordered by id.
    fn list(&self) -> BoxFuture<'_, Result<Vec<AccountInfo>, AccountsError>>;

    /// The account the entry infers with, when one is selected.
    fn active(&self) -> BoxFuture<'_, Result<Option<String>, AccountsError>>;

    fn set_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), AccountsError>>;

    /// Runs the family's device login and stores the account. `pending`
    /// receives what the user must do, once. Dropping the future cancels the
    /// login, even while it waits between polls, and stores nothing.
    fn login<'a>(
        &'a self,
        pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> BoxFuture<'a, Result<AccountInfo, LoginError>>;

    /// Stores an account from material the product supplies.
    fn add(&self, input: AddAccount) -> BoxFuture<'_, Result<AccountInfo, AccountsError>>;

    /// Removes an account other than the active one: the entry keeps
    /// inferring with its active account until another is selected.
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
    /// Removing the active account is refused.
    #[error("the active account cannot be removed; select another account first")]
    Active,
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

impl From<PoolError> for AccountsError {
    fn from(error: PoolError) -> Self {
        match error {
            PoolError::NotFound(id) => Self::NotFound(id),
            PoolError::Store(message) => Self::Store(message),
        }
    }
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

/// How an account names itself: what the user knows it by, and what tells
/// it apart from the entry's other accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLabel {
    pub label: String,
    pub detail: Option<String>,
    /// Derived from the account itself, so that logging in again with the
    /// same account replaces its record.
    pub identity_key: Option<String>,
}

/// An account a family made: its secret document and how it names itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    /// The secret document, in the family's own format.
    pub secret: String,
    pub label: AccountLabel,
}

/// What a family adds to the account operations.
pub trait AccountKit: Send + Sync + 'static {
    fn capability(&self) -> AccountsCapability;

    /// Runs the family's device login, reporting what the user must do to
    /// `pending` once. `None` when the family has no device login.
    fn login<'a>(
        &'a self,
        pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>>;

    /// The account of material the product supplies. `None` when the family
    /// adds no accounts this way.
    fn add(&self, input: AddAccount) -> Option<Result<NewAccount, AccountsError>>;
}

/// The account operations of one entry's pool, with a family's kit.
pub struct Accounts<K> {
    pool: Arc<dyn CredentialPool>,
    kit: K,
    clock: Arc<dyn Clock>,
}

impl<K: AccountKit> Accounts<K> {
    pub fn new(pool: Arc<dyn CredentialPool>, kit: K, clock: Arc<dyn Clock>) -> Self {
        Self { pool, kit, clock }
    }

    /// Stores `account`: an account with the same identity key is replaced,
    /// a new one gets an id derived from its identity, and the first account
    /// of an entry without an active one becomes its active account.
    async fn import(&self, account: NewAccount, source: &str) -> Result<AccountInfo, AccountsError> {
        let NewAccount { secret, label } = account;
        let existing = match &label.identity_key {
            Some(key) => find_by_identity(&*self.pool, key).await?,
            None => None,
        };
        let id = match existing {
            Some(existing) => existing.id,
            None => credential_id_for(label.identity_key.as_deref(), &label.label),
        };
        let meta = AccountMeta {
            id: id.clone(),
            label: label.label,
            detail: label.detail,
            updated_at: self.clock.now(),
            source: source.to_owned(),
            identity_key: label.identity_key,
        };
        let info = meta.info();
        self.pool.write(meta, secret).await?;
        if self.pool.active().await?.is_none() {
            self.pool.set_active(&id).await?;
        }
        Ok(info)
    }
}

impl<K: AccountKit> SubscriptionAccounts for Accounts<K> {
    fn capability(&self) -> AccountsCapability {
        self.kit.capability()
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<AccountInfo>, AccountsError>> {
        Box::pin(async {
            let accounts = self.pool.list().await?;
            Ok(accounts.iter().map(AccountMeta::info).collect())
        })
    }

    fn active(&self) -> BoxFuture<'_, Result<Option<String>, AccountsError>> {
        Box::pin(async { Ok(self.pool.active().await?) })
    }

    fn set_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), AccountsError>> {
        Box::pin(async move { Ok(self.pool.set_active(id).await?) })
    }

    fn login<'a>(
        &'a self,
        pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> BoxFuture<'a, Result<AccountInfo, LoginError>> {
        Box::pin(async move {
            let Some(login) = self.kit.login(pending) else {
                return Err(LoginError::Unsupported);
            };
            let account = login.await?;
            self.import(account, "login:device")
                .await
                .map_err(|error| LoginError::Failed(error.to_string()))
        })
    }

    fn add(&self, input: AddAccount) -> BoxFuture<'_, Result<AccountInfo, AccountsError>> {
        Box::pin(async move {
            let Some(account) = self.kit.add(input) else {
                return Err(AccountsError::Unsupported);
            };
            self.import(account?, "add").await
        })
    }

    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), AccountsError>> {
        Box::pin(async move {
            if self.pool.active().await?.as_deref() == Some(id) {
                return Err(AccountsError::Active);
            }
            if self.pool.meta(id).await?.is_none() {
                return Err(AccountsError::NotFound(id.to_owned()));
            }
            Ok(self.pool.remove(id).await?)
        })
    }
}
