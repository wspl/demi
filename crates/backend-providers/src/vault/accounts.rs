//! The product's rules over a subscription entry's accounts
//! (`providers.md` § Login and publication, `web-api.md` § Subscription
//! accounts): every account comes from a sign-in, selecting an account is
//! explicit, and removing the active account selects the entry's next one,
//! or leaves it with none. A change of accounts invalidates the entry's
//! provider and catalog, so the next request builds its runtime anew, while
//! a running one finishes with its own.

use demi_backend_database::StorageError;
use demi_web_api_protocol::ids::CredentialId;
use demi_web_api_protocol::providers::Accounts;

use super::entries::{EntryCredential, ProviderEntry};
use super::pool::meta;
use crate::llm::assembly::{AssemblyError, ProviderAssembly};

/// What an entry without an account says, in its status and when a request
/// is refused before inference: the user adds an account to use it.
pub const NO_ACCOUNT: &str = "This provider has no account. Add one in Settings to use it.";

/// Why an account operation was refused.
#[derive(Debug, thiserror::Error)]
pub enum AccountRefusal {
    /// The entry does not take accounts this way.
    #[error("{0}")]
    Unsupported(&'static str),
    #[error("No such account")]
    NotFound,
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
}

impl From<StorageError> for AccountRefusal {
    fn from(error: StorageError) -> Self {
        Self::Assembly(error.into())
    }
}

/// The entry's accounts and the active one; for a user who only infers,
/// none.
pub async fn list(
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
pub async fn activate(
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
    assembly.vault().mark_changed(&entry.owner);
    assembly.invalidate(&entry.id).await?;
    Ok(account)
}

/// Removes an account, the active one included, with its quota snapshot,
/// and answers the account now active: the entry's first remaining one when
/// the removed one was active, none when it was the last.
pub async fn remove(
    assembly: &ProviderAssembly,
    entry: &ProviderEntry,
    account: CredentialId,
) -> Result<Option<CredentialId>, AccountRefusal> {
    subscription(entry)?;
    let stored = assembly
        .vault()
        .account(entry.id.clone(), account.clone())
        .await?;
    if stored.is_none() {
        return Err(AccountRefusal::NotFound);
    }
    let active = assembly
        .vault()
        .control()
        .remove_credential(entry.id.clone(), account.clone())
        .await?;
    assembly.quotas().forget_account(&entry.id, &account);
    assembly.vault().mark_changed(&entry.owner);
    assembly.invalidate(&entry.id).await?;
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
