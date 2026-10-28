//! The product's rules over a subscription entry's accounts
//! (`crates/backend-providers/src/vault/logins.rs`):
//!
//! - Every account comes from a sign-in.
//! - Selecting an account is explicit.
//! - Removing the active account selects the entry's next one, or leaves it
//!   with none.
//!
//! A change of accounts invalidates the entry's provider and catalog, so the
//! next request builds its runtime anew, while a running one finishes with its
//! own.

use demi_backend_database::StorageError;
use demi_provider_common::credentials::AccountsError;
use demi_web_api_protocol::ids::CredentialId;
use demi_web_api_protocol::providers::Accounts;

use demi_provider_registry::entry::{EntryCredential, ProviderEntry};
use demi_provider_registry::registry::{LoginAccounts, ProviderRegistry, RegistryError};
use demi_provider_registry::store::StoreError;

use super::entries::Vault;
use super::pool::meta;

/// Why an account operation was refused.
#[derive(Debug, thiserror::Error)]
pub enum AccountRefusal {
    /// The entry does not take accounts this way.
    #[error("{0}")]
    Unsupported(&'static str),
    #[error("No such account")]
    NotFound,
    /// An order that does not name every account of the entry once.
    #[error("credentialIds: name every account of this provider once")]
    Order,
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// What the family keeps of the account outside its record could not
    /// be deleted, so the account stays.
    #[error("The account could not be removed: {0}")]
    Forget(AccountsError),
}

impl From<StorageError> for AccountRefusal {
    fn from(error: StorageError) -> Self {
        Self::Registry(StoreError::new(error).into())
    }
}

impl From<StoreError> for AccountRefusal {
    fn from(error: StoreError) -> Self {
        Self::Registry(error.into())
    }
}

/// The entry's accounts and the active one; for a user who only infers,
/// none.
pub async fn list(
    vault: &Vault,
    entry: &ProviderEntry,
    disclose: bool,
) -> Result<Accounts, AccountRefusal> {
    subscription(entry)?;
    if !disclose {
        return Ok(Accounts {
            accounts: Vec::new(),
            active: None,
        });
    }
    let records = vault.accounts(entry.id.clone()).await?;
    Ok(Accounts {
        accounts: records.iter().map(|record| meta(record).info()).collect(),
        active: entry.active().cloned(),
    })
}

/// Makes `account` the one the entry infers with. The next request builds
/// its runtime from the account's provider; a running one finishes with its
/// own.
pub async fn activate(
    vault: &Vault,
    provider_registry: &ProviderRegistry,
    entry: &ProviderEntry,
    account: CredentialId,
) -> Result<CredentialId, AccountRefusal> {
    subscription(entry)?;
    let selected = vault
        .set_active_account(&entry.id, account.clone())
        .await?;
    if !selected {
        return Err(AccountRefusal::NotFound);
    }
    vault.mark_changed(&entry.owner);
    provider_registry.invalidate(&entry.id).await?;
    Ok(account)
}

/// Puts the entry's accounts in the order `accounts` names them, which In
/// Order follows and the list shows; it names every account once.
pub async fn reorder(
    vault: &Vault,
    entry: &ProviderEntry,
    accounts: Vec<CredentialId>,
) -> Result<(), AccountRefusal> {
    subscription(entry)?;
    let ordered = vault
        .control()
        .set_credential_order(entry.id.clone(), accounts)
        .await?;
    if !ordered {
        return Err(AccountRefusal::Order);
    }
    vault.mark_changed(&entry.owner);
    Ok(())
}

/// Removes an account, the active one included, with its quota snapshot,
/// and answers the account now active: the entry's first remaining one when
/// the removed one was active, none when it was the last.
pub async fn remove(
    vault: &Vault,
    provider_registry: &ProviderRegistry,
    entry: &ProviderEntry,
    account: CredentialId,
) -> Result<Option<CredentialId>, AccountRefusal> {
    subscription(entry)?;
    let stored = vault
        .account(entry.id.clone(), account.clone())
        .await?;
    if stored.is_none() {
        return Err(AccountRefusal::NotFound);
    }
    // The family's own keeping of the account goes first, so a failure
    // leaves the account whole (`crates/provider-claude-code/src/account.rs`).
    if let LoginAccounts::Machine(accounts) = provider_registry.login_accounts(entry).await? {
        accounts
            .forget(account.as_str())
            .await
            .map_err(AccountRefusal::Forget)?;
    }
    let active = vault
        .remove_account(entry, account.clone())
        .await?;
    provider_registry.quotas().forget_account(&entry.id, &account);
    vault.mark_changed(&entry.owner);
    provider_registry.invalidate(&entry.id).await?;
    Ok(active)
}

fn subscription(entry: &ProviderEntry) -> Result<(), AccountRefusal> {
    match entry.credential {
        EntryCredential::Subscription { .. } => Ok(()),
        EntryCredential::ApiKey(_) => Err(AccountRefusal::Unsupported(
            "This provider does not use subscription accounts",
        )),
    }
}
