//! The vault's credential pool
//! (`crates/provider-common/src/credentials/pool.rs`): one entry's accounts as
//! records of the control store.
//!
//! Each secret is sealed to its row and versioned by every write, with the
//! vault's refresh turns. A pool is bound to its entry, so a provider given it
//! cannot reach another entry's accounts.

use demi_backend_database::StorageError;
use demi_backend_database::providers::{CredentialRow, CredentialWrite};
use demi_provider_common::credentials::{
    AccountDocument, AccountMeta, CredentialPool, PoolError, RefreshPermit, Revision,
};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};
use futures_util::future::BoxFuture;

use super::entries::Vault;
use super::seal;

/// The accounts of one entry.
pub struct VaultCredentialPool {
    vault: Vault,
    provider: ProviderId,
}

impl VaultCredentialPool {
    pub fn new(vault: Vault, provider: ProviderId) -> Self {
        Self { vault, provider }
    }
}

/// An account's public metadata from its record.
pub fn meta(row: &CredentialRow) -> AccountMeta {
    AccountMeta {
        id: row.id.as_str().to_owned(),
        label: row.label.clone(),
        detail: row.detail.clone(),
        updated_at: row.updated_at,
        source: row.source.clone(),
        identity_key: row.identity_key.clone(),
        signed_out: row.signed_out,
    }
}

fn store(error: StorageError) -> PoolError {
    // A storage error names tables and columns, never a secret.
    PoolError::Store(error.to_string())
}

impl CredentialPool for VaultCredentialPool {
    fn list(&self) -> BoxFuture<'_, Result<Vec<AccountMeta>, PoolError>> {
        Box::pin(async {
            let rows = self
                .vault
                .accounts(self.provider.clone())
                .await
                .map_err(store)?;
            Ok(rows.iter().map(meta).collect())
        })
    }

    fn meta<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<AccountMeta>, PoolError>> {
        Box::pin(async move {
            // An id that is no account's names nothing.
            let Ok(id) = CredentialId::try_from(id) else {
                return Ok(None);
            };
            let row = self
                .vault
                .account(self.provider.clone(), id)
                .await
                .map_err(store)?;
            Ok(row.as_ref().map(meta))
        })
    }

    fn active(&self) -> BoxFuture<'_, Result<Option<String>, PoolError>> {
        Box::pin(async {
            let row = self
                .vault
                .control()
                .provider(self.provider.clone())
                .await
                .map_err(store)?;
            Ok(row
                .and_then(|row| row.active)
                .map(CredentialId::into_string))
        })
    }

    fn set_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>> {
        Box::pin(async move {
            let not_found = || PoolError::NotFound(id.to_owned());
            let account = CredentialId::try_from(id).map_err(|_| not_found())?;
            let selected = self
                .vault
                .set_active_account(&self.provider, account)
                .await
                .map_err(store)?;
            if !selected {
                return Err(not_found());
            }
            self.vault.mark_entry_changed(&self.provider).await;
            Ok(())
        })
    }

    /// Stores the account, and selects it when the entry has no active
    /// account, in one transaction; the record's time is the write's.
    fn write(
        &self,
        meta: AccountMeta,
        secret: Option<String>,
    ) -> BoxFuture<'_, Result<(), PoolError>> {
        Box::pin(async move {
            let id = CredentialId::try_from(meta.id)
                .map_err(|error| PoolError::Store(error.to_string()))?;
            let account = CredentialWrite {
                secret: secret.map(|secret| self.vault.seal_secret(&self.provider, &id, &secret)),
                id,
                identity_key: meta.identity_key,
                label: meta.label,
                detail: meta.detail,
                source: meta.source,
            };
            let stored = self
                .vault
                .write_account(&self.provider, account)
                .await
                .map_err(store)?;
            if !stored {
                return Err(PoolError::Store(
                    "the provider entry no longer exists".into(),
                ));
            }
            self.vault.mark_entry_changed(&self.provider).await;
            Ok(())
        })
    }

    fn sign_out<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>> {
        Box::pin(async move {
            let Ok(account) = CredentialId::try_from(id) else {
                return Ok(());
            };
            let changed = self
                .vault
                .control()
                .sign_out_credential(self.provider.clone(), account)
                .await
                .map_err(store)?;
            if changed {
                self.vault.mark_entry_changed(&self.provider).await;
            }
            Ok(())
        })
    }

    fn document(&self, id: &str) -> Box<dyn AccountDocument> {
        Box::new(VaultDocument {
            vault: self.vault.clone(),
            provider: self.provider.clone(),
            account: CredentialId::try_from(id).ok(),
            name: format!("account {id}"),
        })
    }

    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>> {
        Box::pin(async move {
            let Ok(account) = CredentialId::try_from(id) else {
                return Ok(());
            };
            self.vault
                .control()
                .remove_credential(self.provider.clone(), account)
                .await
                .map_err(store)?;
            self.vault.mark_entry_changed(&self.provider).await;
            Ok(())
        })
    }
}

/// One account's secret document in the vault.
struct VaultDocument {
    vault: Vault,
    provider: ProviderId,
    /// `None` for an id that is no account's, whose document is empty.
    account: Option<CredentialId>,
    name: String,
}

impl AccountDocument for VaultDocument {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<Revision>, PoolError>> {
        Box::pin(async {
            let Some(account) = &self.account else {
                return Ok(None);
            };
            let Some(row) = self
                .vault
                .account(self.provider.clone(), account.clone())
                .await
                .map_err(store)?
            else {
                return Ok(None);
            };
            let Some(secret) = &row.secret else {
                return Ok(None);
            };
            // A secret that does not open is corrupt: requests with the
            // account fail with an authentication error, and nothing
            // replaces it.
            let document = self
                .vault
                .key()
                .open(seal::Row::Secret(&self.provider, account), secret)
                .map_err(|_| {
                    PoolError::Store(format!("the secret of {} does not open", self.name))
                })?;
            let text = String::from_utf8(document).map_err(|_| {
                PoolError::Store(format!("the secret of {} is not UTF-8", self.name))
            })?;
            Ok(Some(Revision {
                text,
                version: row.version,
            }))
        })
    }

    fn replace(
        &self,
        text: Option<String>,
        version: u64,
    ) -> BoxFuture<'_, Result<bool, PoolError>> {
        Box::pin(async move {
            let Some(account) = &self.account else {
                return Ok(false);
            };
            let sealed = text.map(|text| self.vault.seal_secret(&self.provider, account, &text));
            let replaced = self
                .vault
                .control()
                .replace_credential_secret(self.provider.clone(), account.clone(), sealed, version)
                .await
                .map_err(store)?;
            if replaced {
                self.vault.mark_entry_changed(&self.provider).await;
            }
            Ok(replaced)
        })
    }

    fn refresh_turn(&self) -> BoxFuture<'_, RefreshPermit> {
        // A provider id is a UUID, which holds no `/`, so no two accounts of
        // any entries share a key.
        let key = format!(
            "{}/{}",
            self.provider,
            self.account.as_ref().map_or("", CredentialId::as_str)
        );
        Box::pin(async move { self.vault.gates().turn(&key).await })
    }
}
