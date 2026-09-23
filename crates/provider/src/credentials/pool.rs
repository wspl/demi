//! The credential pool contract (`providers.md` § The credential pool
//! contract): the accounts of one entry, which of them is active, and each
//! account's secret document with its version. A provider crate never knows
//! where the pool keeps them.

use demi_core::{AccountInfo, Timestamp};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};

use crate::credentials::RefreshPermit;

/// An account's public metadata; never token material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountMeta {
    /// The account's id within its entry, such as `cred-3f2a9c01d4e5b6a7`.
    pub id: String,
    /// What the user knows the account by, such as an email address.
    pub label: String,
    /// A second line, such as the kind of sign-in.
    pub detail: Option<String>,
    /// When the account was last stored.
    pub updated_at: Timestamp,
    /// How the account arrived, such as `login:device`.
    pub source: String,
    /// What tells this account apart from the entry's others, derived by its
    /// family from the account itself, so that logging in again with the
    /// same account replaces its record.
    pub identity_key: Option<String>,
}

impl AccountMeta {
    /// The account as the browser sees it.
    pub fn info(&self) -> AccountInfo {
        AccountInfo {
            id: self.id.clone(),
            label: self.label.clone(),
            detail: self.detail.clone(),
            updated_at: Some(self.updated_at),
        }
    }
}

/// Why a pool operation failed. The message never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PoolError {
    #[error("no account {0}")]
    NotFound(String),
    /// Whatever keeps the pool failed.
    #[error("{0}")]
    Store(String),
}

/// One revision of an account's secret document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// The document in its family's format.
    pub text: String,
    /// Changes with every write of the document; only compared for equality.
    pub version: u64,
}

/// One account's secret document. Its text is the family's own format, which
/// the pool never looks inside.
pub trait AccountDocument: Send + Sync {
    /// Names the document in messages; never secret.
    fn name(&self) -> &str;

    /// The current revision, or `None` when the account has no document.
    fn read(&self) -> BoxFuture<'_, Result<Option<Revision>, PoolError>>;

    /// Stores `text` if the document is still at `version`; `false` when
    /// another writer stored first, and the caller then reads again and uses
    /// what it finds.
    fn replace(&self, text: String, version: u64) -> BoxFuture<'_, Result<bool, PoolError>>;

    /// Waits until no other refresh of this account runs in this process. A
    /// refresh token is spent once, so refreshes of one account take turns and
    /// the later one finds the earlier one's tokens. The turn ends when the
    /// permit is dropped, whether the refresh succeeded, failed or was
    /// cancelled.
    fn refresh_turn(&self) -> BoxFuture<'_, RefreshPermit>;
}

/// The accounts of one provider entry and which of them is active. The pool
/// a provider receives is bound to its entry, so a provider cannot reach
/// another entry's accounts.
pub trait CredentialPool: Send + Sync {
    /// The entry's accounts, ordered by id.
    fn list(&self) -> BoxFuture<'_, Result<Vec<AccountMeta>, PoolError>>;

    fn meta<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<AccountMeta>, PoolError>>;

    /// The account the entry infers with, when one is selected.
    fn active(&self) -> BoxFuture<'_, Result<Option<String>, PoolError>>;

    /// Selects the account the entry infers with; `NotFound` for an account
    /// the entry does not hold.
    fn set_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>>;

    /// Inserts the account, or replaces the one with its id, with its secret
    /// document.
    fn write(&self, meta: AccountMeta, secret: String) -> BoxFuture<'_, Result<(), PoolError>>;

    /// The secret document of the account `id`, which need not exist yet.
    fn document(&self, id: &str) -> Box<dyn AccountDocument>;

    /// Removes the account and its document; an active selection of it goes
    /// with it.
    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>>;
}

/// The account of `pool` whose identity key is `identity_key`.
pub async fn find_by_identity(
    pool: &dyn CredentialPool,
    identity_key: &str,
) -> Result<Option<AccountMeta>, PoolError> {
    let accounts = pool.list().await?;
    Ok(accounts
        .into_iter()
        .find(|account| account.identity_key.as_deref() == Some(identity_key)))
}

/// The id of a new account: `cred-` and the first 16 hexadecimal digits of
/// the SHA-256 of its identity key, or of its label when it has none, so the
/// same account gets the same id wherever it is imported.
pub fn credential_id_for(identity_key: Option<&str>, label: &str) -> String {
    let basis = identity_key.filter(|key| !key.is_empty()).unwrap_or(label);
    let digest = Sha256::digest(basis.as_bytes());
    let mut id = String::from("cred-");
    for byte in &digest[..8] {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}
