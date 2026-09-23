//! The vault's credential pool (`providers.md` § The credential pool
//! contract): one entry's accounts as records of the control store, each
//! secret sealed to its row and versioned by every write, with the vault's
//! refresh turns. A pool is bound to its entry, so a provider given it cannot
//! reach another entry's accounts.

use demi_provider::credentials::{AccountDocument, AccountMeta, CredentialPool, PoolError, RefreshPermit, Revision};
use demi_web_api::ids::{CredentialId, ProviderId};
use futures_util::future::BoxFuture;

use super::entries::Vault;
use super::seal;
use crate::storage::StorageError;
use crate::storage::providers::{CredentialRow, CredentialWrite};

/// The accounts of one entry.
pub(crate) struct VaultCredentialPool {
    vault: Vault,
    provider: ProviderId,
}

impl VaultCredentialPool {
    pub(crate) fn new(vault: Vault, provider: ProviderId) -> Self {
        Self { vault, provider }
    }
}

/// An account's public metadata from its record.
pub(crate) fn meta(row: &CredentialRow) -> AccountMeta {
    AccountMeta {
        id: row.id.as_str().to_owned(),
        label: row.label.clone(),
        detail: row.detail.clone(),
        updated_at: row.updated_at,
        source: row.source.clone(),
        identity_key: row.identity_key.clone(),
    }
}

fn store(error: StorageError) -> PoolError {
    // A storage error names tables and columns, never a secret.
    PoolError::Store(error.to_string())
}

impl CredentialPool for VaultCredentialPool {
    fn list(&self) -> BoxFuture<'_, Result<Vec<AccountMeta>, PoolError>> {
        Box::pin(async {
            let rows = self.vault.accounts(self.provider.clone()).await.map_err(store)?;
            Ok(rows.iter().map(meta).collect())
        })
    }

    fn meta<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<AccountMeta>, PoolError>> {
        Box::pin(async move {
            // An id that is no account's names nothing.
            let Ok(id) = CredentialId::try_from(id) else {
                return Ok(None);
            };
            let row = self.vault.account(self.provider.clone(), id).await.map_err(store)?;
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
            Ok(row.and_then(|row| row.active).map(CredentialId::into_string))
        })
    }

    fn set_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>> {
        Box::pin(async move {
            let not_found = || PoolError::NotFound(id.to_owned());
            let account = CredentialId::try_from(id).map_err(|_| not_found())?;
            let selected = self
                .vault
                .control()
                .set_active_credential(self.provider.clone(), account)
                .await
                .map_err(store)?;
            if selected { Ok(()) } else { Err(not_found()) }
        })
    }

    /// Stores the account, and selects it when the entry has no active
    /// account, in one transaction; the record's time is the write's.
    fn write(&self, meta: AccountMeta, secret: String) -> BoxFuture<'_, Result<(), PoolError>> {
        Box::pin(async move {
            let id = CredentialId::try_from(meta.id).map_err(|error| PoolError::Store(error.to_string()))?;
            let account = CredentialWrite {
                secret: self.vault.seal_secret(&self.provider, &id, &secret),
                id,
                identity_key: meta.identity_key,
                label: meta.label,
                detail: meta.detail,
                source: meta.source,
            };
            let stored = self
                .vault
                .control()
                .write_credential(self.provider.clone(), account)
                .await
                .map_err(store)?;
            if stored {
                Ok(())
            } else {
                Err(PoolError::Store("the provider entry no longer exists".into()))
            }
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
                .map_err(store)
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
            // A secret that does not open is corrupt: requests with the
            // account fail with an authentication error, and nothing
            // replaces it.
            let document = self
                .vault
                .key()
                .open(seal::Row::Secret(&self.provider, account), &row.secret)
                .map_err(|_| PoolError::Store(format!("the secret of {} does not open", self.name)))?;
            let text = String::from_utf8(document)
                .map_err(|_| PoolError::Store(format!("the secret of {} is not UTF-8", self.name)))?;
            Ok(Some(Revision {
                text,
                version: row.version,
            }))
        })
    }

    fn replace(&self, text: String, version: u64) -> BoxFuture<'_, Result<bool, PoolError>> {
        Box::pin(async move {
            let Some(account) = &self.account else {
                return Ok(false);
            };
            let sealed = self.vault.seal_secret(&self.provider, account, &text);
            self.vault
                .control()
                .replace_credential_secret(self.provider.clone(), account.clone(), sealed, version)
                .await
                .map_err(store)
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use demi_core::Timestamp;
    use demi_provider::credentials::{MemoryCredentialPool, SecretDocument, renew};
    use demi_web_api::settings::InstanceMode;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::storage::control::{ControlService, testing};
    use crate::vault::seal::VaultKey;

    fn account(id: &str) -> AccountMeta {
        AccountMeta {
            id: id.into(),
            label: format!("{id}@example.test"),
            detail: None,
            updated_at: Timestamp::UNIX_EPOCH,
            source: "login:device".into(),
            identity_key: Some(id.into()),
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Tokens {
        refresh: String,
    }

    impl SecretDocument for Tokens {}

    #[tokio::test]
    async fn an_account_is_a_sealed_record_refreshed_only_over_the_version_it_was_read_at() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
            .await
            .unwrap();
        let master = testing::master(&control).await;
        let vault = Vault::new(control.clone(), VaultKey::new([3; 32]), InstanceMode::Isolated);
        let staged = MemoryCredentialPool::new();
        let entry = vault
            .create_subscription(master.id.clone(), "codex".into(), "Codex".into(), &staged)
            .await
            .unwrap()
            .unwrap();
        let pool = vault.pool(entry.id.clone());
        assert_eq!(pool.active().await, Ok(None));
        pool.write(account("a"), r#"{"refresh":"one"}"#.into()).await.unwrap();
        // The first account of an entry without an active one becomes it, in
        // the same write.
        assert_eq!(pool.active().await.unwrap().as_deref(), Some("a"));
        let rows = control.credentials(entry.id.clone()).await.unwrap();
        assert!(!rows[0].secret.windows(3).any(|window| window == b"one"));

        // Two refreshers read one revision; the second write finds a newer one.
        let document = pool.document("a");
        let first = document.read().await.unwrap().unwrap();
        let second = document.read().await.unwrap().unwrap();
        assert_eq!(first.text, r#"{"refresh":"one"}"#);
        assert!(
            document
                .replace(r#"{"refresh":"two"}"#.into(), first.version)
                .await
                .unwrap()
        );
        assert!(
            !document
                .replace(r#"{"refresh":"lost"}"#.into(), second.version)
                .await
                .unwrap()
        );
        assert_eq!(document.read().await.unwrap().unwrap().text, r#"{"refresh":"two"}"#);

        // Concurrent renewals of one account ask the vendor once, and the
        // second uses what the first stored.
        let asked = AtomicUsize::new(0);
        let refresh = |tokens: Tokens| {
            let asked = &asked;
            async move {
                asked.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                Ok::<_, String>(Tokens {
                    refresh: format!("{}+", tokens.refresh),
                })
            }
        };
        let still_due = |tokens: &Tokens| tokens.refresh == "two";
        let (one, another) = (pool.document("a"), pool.document("a"));
        let (left, right) = tokio::join!(
            renew::<Tokens, String, _, _>(&*one, still_due, refresh),
            renew::<Tokens, String, _, _>(&*another, still_due, refresh),
        );
        assert_eq!(
            (left.unwrap().refresh, right.unwrap().refresh),
            ("two+".to_owned(), "two+".to_owned())
        );
        assert_eq!(asked.load(Ordering::SeqCst), 1);

        // Another entry's pool reaches none of these accounts.
        let other = vault
            .create_subscription(master.id.clone(), "grok-build".into(), "Grok".into(), &staged)
            .await
            .unwrap()
            .unwrap();
        let foreign = vault.pool(other.id.clone());
        assert_eq!(foreign.document("a").read().await, Ok(None));
        assert_eq!(foreign.list().await, Ok(Vec::new()));
        // A secret moved into another entry's row does not open there.
        foreign.write(account("a"), "{}".into()).await.unwrap();
        testing::execute(
            &control,
            "UPDATE provider_credentials SET secret = (SELECT secret FROM provider_credentials
               WHERE provider_id = ?1 AND id = 'a') WHERE provider_id = ?2",
            vec![entry.id.to_string(), other.id.to_string()],
        )
        .await;
        assert!(matches!(foreign.document("a").read().await, Err(PoolError::Store(_))));

        pool.write(account("b"), "{}".into()).await.unwrap();
        assert_eq!(
            pool.set_active("missing").await,
            Err(PoolError::NotFound("missing".into()))
        );
        pool.set_active("b").await.unwrap();
        pool.remove("b").await.unwrap();
        assert_eq!(pool.active().await, Ok(None));
        vault.delete(entry.id.clone()).await.unwrap();
        assert_eq!(control.credentials(entry.id.clone()).await.unwrap(), Vec::new());
        control.close().await.unwrap();
    }
}
