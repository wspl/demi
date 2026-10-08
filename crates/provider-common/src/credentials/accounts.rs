//! The account operations of a subscription family (`providers.md` §
//! Credential vault): listing, selecting, logging in and removing the
//! accounts of the provider's entry. One implementation, [`Accounts`], serves
//! every family over its credential pool; a family supplies only what differs,
//! its [`AccountKit`]: how it logs in, and how an account names itself.

use std::future::Future;
use std::sync::Arc;

use demi_shared_types::{AccountInfo, Clock, LoginPending};
use futures_util::future::BoxFuture;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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

    /// Runs the family's login and stores the account. The login reports what
    /// the user must do to `io.pending`, takes the codes the user pastes from
    /// `io.codes`, and once `io.stop` fires ends with [`LoginError::Stopped`]
    /// as soon as it has released what it holds. Dropping the future cancels
    /// it as well. A login that does not complete stores nothing.
    fn login<'a>(&'a self, io: LoginIo<'a>) -> BoxFuture<'a, Result<AccountInfo, LoginError>>;

    /// Removes an account other than the active one: the entry keeps
    /// inferring with its active account until another is selected.
    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), AccountsError>>;
}

/// How a family adds an account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccountsCapability {
    /// The family's login; none for a family whose provider cannot log in,
    /// such as one built without the machine its login runs on.
    pub login: Option<LoginKind>,
}

/// What the user does in a family's login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginKind {
    /// A device login: the user confirms a one-time code at the vendor's page
    /// on any device.
    Device,
    /// The user signs in at the vendor's page on any device and pastes the
    /// code the page shows back into Demi, which hands it to the login.
    PastedCode,
}

/// What a login works with: where it reports what the user must do, the
/// codes the user pastes, and when to stop.
pub struct LoginIo<'a> {
    pub pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    /// The codes the user pastes, in order; a login that takes none leaves
    /// them.
    pub codes: mpsc::UnboundedReceiver<Secret>,
    /// Fires when the login is cancelled or its time is up.
    pub stop: CancellationToken,
}

impl LoginIo<'_> {
    /// Runs `login` until it ends or the login is stopped, for a login that
    /// holds nothing that needs releasing, such as one that only polls the
    /// vendor.
    pub async fn until_stopped<T>(
        stop: &CancellationToken,
        login: impl Future<Output = Result<T, LoginError>>,
    ) -> Result<T, LoginError> {
        stop.run_until_cancelled(login)
            .await
            .unwrap_or(Err(LoginError::Stopped))
    }
}

/// Why an account operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountsError {
    #[error("no account {0}")]
    NotFound(String),
    /// Removing the active account is refused.
    #[error("the active account cannot be removed; select another account first")]
    Active,
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
    /// The family has no login.
    #[error("this provider has no login")]
    Unsupported,
    /// The vendor does not offer the login, such as Codex answering 404.
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
    /// The login was stopped, and has released what it held.
    #[error("the login was stopped")]
    Stopped,
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

    /// Runs the family's login with `io`, by the rules of
    /// [`SubscriptionAccounts::login`]. `None` when the family has no login.
    fn login<'a>(
        &'a self,
        io: LoginIo<'a>,
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>>;
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

    /// Stores the account a login made: an account with the same identity key
    /// is replaced, keeping its id; a new one gets an id derived from its
    /// identity, and the first account of an entry without an active one
    /// becomes its active account.
    async fn import(&self, account: NewAccount) -> Result<AccountInfo, LoginError> {
        let stored = |error: PoolError| LoginError::Failed(AccountsError::from(error).to_string());
        let NewAccount { secret, label } = account;
        let existing = match &label.identity_key {
            Some(key) => find_by_identity(&*self.pool, key).await.map_err(stored)?,
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
            source: match self.kit.capability().login {
                Some(LoginKind::PastedCode) => "login:code",
                Some(LoginKind::Device) | None => "login:device",
            }
            .to_owned(),
            identity_key: label.identity_key,
        };
        let info = meta.info();
        self.pool.write(meta, secret).await.map_err(stored)?;
        if self.pool.active().await.map_err(stored)?.is_none() {
            self.pool.set_active(&id).await.map_err(stored)?;
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

    fn login<'a>(&'a self, io: LoginIo<'a>) -> BoxFuture<'a, Result<AccountInfo, LoginError>> {
        Box::pin(async move {
            let Some(login) = self.kit.login(io) else {
                return Err(LoginError::Unsupported);
            };
            let account = login.await?;
            self.import(account).await
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
