//! The product's rules over a subscription entry's accounts
//! (`providers.md` § Login and publication, `web-api.md` § Subscription
//! accounts): a setup-token import creates the Claude Code entry or adds an
//! account to it, selecting an account is explicit, and the active account
//! cannot be removed. A change of accounts invalidates the entry's provider
//! and catalog.

use std::sync::Arc;

use demi_core::AccountInfo;
use demi_provider::Secret;
use demi_provider::credentials::{AccountsError, AddAccount, MemoryCredentialPool};
use demi_web_api::ids::{CredentialId, UserId};
use demi_web_api::providers::Accounts;

use super::entries::{EntryCredential, ProviderEntry};
use super::pool::meta;
use crate::llm::assembly::{AssemblyError, ProviderAssembly};
use crate::storage::StorageError;

/// The family whose accounts are setup tokens.
pub(crate) const SETUP_TOKEN_FAMILY: &str = "claude-code";

/// Why an account operation was refused.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AccountRefusal {
    /// The owner already holds the family's subscription entry.
    #[error("Add this token to the existing Claude Code provider")]
    Exists,
    /// The entry does not take accounts this way.
    #[error("{0}")]
    Unsupported(&'static str),
    #[error("No such account")]
    NotFound,
    #[error("Select another account before removing the active one, or delete the provider")]
    Active,
    /// A fixed message, since the family's own may quote the token.
    #[error("The setup token could not be imported")]
    TokenImportFailed,
    /// The account store failed.
    #[error("{0}")]
    Store(String),
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
}

impl From<StorageError> for AccountRefusal {
    fn from(error: StorageError) -> Self {
        Self::Assembly(error.into())
    }
}

/// Creates `owner`'s Claude Code entry from a setup token: the token becomes
/// the entry's first account, published with the entry in one transaction.
pub(crate) async fn import_setup_token(
    assembly: &ProviderAssembly,
    owner: UserId,
    label: String,
    token: String,
) -> Result<ProviderEntry, AccountRefusal> {
    if assembly.families().get(SETUP_TOKEN_FAMILY).is_none() {
        return Err(AccountRefusal::Unsupported("Setup-token import is unavailable"));
    }
    let entries = assembly.vault().entries(owner.clone()).await?;
    if entries.iter().any(|entry| entry.family == SETUP_TOKEN_FAMILY) {
        return Err(AccountRefusal::Exists);
    }
    let staged = MemoryCredentialPool::new();
    let id = uuid::Uuid::new_v4().to_string();
    let provider = assembly.detached(SETUP_TOKEN_FAMILY, &id, &label, Arc::new(staged.clone()))?;
    add(&*provider, token).await?;
    assembly
        .vault()
        .create_subscription(owner, SETUP_TOKEN_FAMILY.to_owned(), label, &staged)
        .await?
        .ok_or(AccountRefusal::Exists)
}

/// Adds a setup token's account to `entry`.
pub(crate) async fn add_token(
    assembly: &ProviderAssembly,
    entry: &ProviderEntry,
    token: String,
) -> Result<AccountInfo, AccountRefusal> {
    subscription(entry)?;
    let provider = assembly.provider_for(entry).await?;
    let account = add(&*provider, token).await?;
    assembly.invalidate(&entry.id).await?;
    Ok(account)
}

/// Stores the token's account through the provider's own account
/// operations, which know the family's secret document.
async fn add(provider: &dyn demi_provider::Provider, token: String) -> Result<AccountInfo, AccountRefusal> {
    let accounts = provider
        .accounts()
        .filter(|accounts| accounts.capability().add)
        .ok_or(AccountRefusal::Unsupported("Use device login for this provider"))?;
    let secret = Secret::try_from(token).map_err(|_| AccountRefusal::TokenImportFailed)?;
    match accounts.add(AddAccount::SetupToken(secret)).await {
        Ok(account) => Ok(account),
        Err(AccountsError::Unsupported) => Err(AccountRefusal::Unsupported("Use device login for this provider")),
        Err(AccountsError::Store(message)) => Err(AccountRefusal::Store(message)),
        // Whatever else the family says may quote the token.
        Err(AccountsError::Invalid(_) | AccountsError::NotFound(_) | AccountsError::Active) => {
            Err(AccountRefusal::TokenImportFailed)
        }
    }
}

/// The entry's accounts and the active one; for a user who only infers,
/// none.
pub(crate) async fn list(
    assembly: &ProviderAssembly,
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
    let records = assembly.vault().accounts(entry.id.clone()).await?;
    Ok(Accounts {
        accounts: records.iter().map(|record| meta(record).info()).collect(),
        active: entry.active().cloned(),
    })
}

/// Makes `account` the one the entry infers with. The next request builds
/// its runtime from the account's provider; a running one finishes with its
/// own.
pub(crate) async fn activate(
    assembly: &ProviderAssembly,
    entry: &ProviderEntry,
    account: CredentialId,
) -> Result<CredentialId, AccountRefusal> {
    subscription(entry)?;
    let selected = assembly
        .vault()
        .control()
        .set_active_credential(entry.id.clone(), account.clone())
        .await?;
    if !selected {
        return Err(AccountRefusal::NotFound);
    }
    assembly.invalidate(&entry.id).await?;
    Ok(account)
}

/// Removes an account other than the active one, with its quota snapshot.
pub(crate) async fn remove(
    assembly: &ProviderAssembly,
    entry: &ProviderEntry,
    account: CredentialId,
) -> Result<(), AccountRefusal> {
    subscription(entry)?;
    if entry.active() == Some(&account) {
        return Err(AccountRefusal::Active);
    }
    let stored = assembly.vault().account(entry.id.clone(), account.clone()).await?;
    if stored.is_none() {
        return Err(AccountRefusal::NotFound);
    }
    assembly
        .vault()
        .control()
        .remove_credential(entry.id.clone(), account.clone())
        .await?;
    assembly.quotas().forget_account(&entry.id, &account);
    assembly.invalidate(&entry.id).await?;
    Ok(())
}

fn subscription(entry: &ProviderEntry) -> Result<(), AccountRefusal> {
    match entry.credential {
        EntryCredential::Subscription { .. } => Ok(()),
        EntryCredential::ApiKey(_) => Err(AccountRefusal::Unsupported(
            "This provider does not use subscription accounts",
        )),
    }
}
