//! The vault as the provider registry's store: the entries' accounts and
//! quota snapshots in their credential records, the catalogs in the control
//! store's `model_catalogs` table, and each change marked on the pages that
//! show the entry.

use std::sync::Arc;

use demi_backend_database::providers::CredentialRow;
use demi_provider_common::credentials::CredentialPool;
use demi_provider_registry::store::{RegistryStore, StoreError, StoredAccount};
use demi_shared_types::{CatalogRecord, QuotaSnapshot};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};
use futures_util::FutureExt;
use futures_util::future::BoxFuture;

use super::entries::Vault;

/// The account a credential record holds, its secret left out.
pub fn stored(row: CredentialRow) -> StoredAccount {
    StoredAccount {
        id: row.id,
        identity_key: row.identity_key,
        label: row.label,
        detail: row.detail,
        source: row.source,
        quota: row.quota,
        updated_at: row.updated_at,
        signed_out: row.signed_out,
    }
}

impl RegistryStore for Vault {
    fn accounts(&self, entry: ProviderId) -> BoxFuture<'_, Result<Vec<StoredAccount>, StoreError>> {
        async move {
            let rows = Vault::accounts(self, entry).await.map_err(StoreError::new)?;
            Ok(rows.into_iter().map(stored).collect())
        }
        .boxed()
    }

    fn account(&self, entry: ProviderId, account: CredentialId) -> BoxFuture<'_, Result<Option<StoredAccount>, StoreError>> {
        async move {
            let row = Vault::account(self, entry, account).await.map_err(StoreError::new)?;
            Ok(row.map(stored))
        }
        .boxed()
    }

    fn pool(&self, entry: ProviderId) -> Arc<dyn CredentialPool> {
        Vault::pool(self, entry)
    }

    fn set_quota(
        &self,
        entry: ProviderId,
        account: CredentialId,
        snapshot: Arc<QuotaSnapshot>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        async move {
            self.control()
                .set_credential_quota(entry, account, snapshot)
                .await
                .map_err(StoreError::new)
        }
        .boxed()
    }

    fn entry_changed(&self, entry: ProviderId) -> BoxFuture<'_, ()> {
        async move { self.mark_entry_changed(&entry).await }.boxed()
    }

    fn catalog(&self, entry: ProviderId) -> BoxFuture<'_, Result<Option<CatalogRecord>, StoreError>> {
        async move { self.control().catalog_record(entry).await.map_err(StoreError::new) }.boxed()
    }

    fn put_catalog(&self, entry: ProviderId, record: CatalogRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        async move {
            self.control()
                .put_catalog_record(entry, record)
                .await
                .map_err(StoreError::new)
        }
        .boxed()
    }

    fn delete_catalog(&self, entry: ProviderId) -> BoxFuture<'_, Result<(), StoreError>> {
        async move { self.control().delete_catalog_record(entry).await.map_err(StoreError::new) }.boxed()
    }
}
