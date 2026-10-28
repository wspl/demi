//! What the registry keeps beyond the process, which the program that uses it
//! stores: its entries' subscription accounts, with each account's credential
//! pool and quota snapshot, and each entry's last catalog.
//!
//! The registry reads and writes them through [`RegistryStore`] alone, and
//! tells the program through it when what a page shows of an entry changed.

use std::sync::Arc;

use demi_provider_common::credentials::{AccountMeta, CredentialPool};
use demi_shared_types::{CatalogRecord, QuotaSnapshot, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};
use futures_util::future::BoxFuture;

/// A failure of the program's storage, which the registry passes on.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StoreError(Box<dyn std::error::Error + Send + Sync>);

impl StoreError {
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

/// A subscription account as stored, its secret left out.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAccount {
    pub id: CredentialId,
    /// What tells the account apart from the entry's others
    /// ([`AccountMeta::identity_key`]).
    pub identity_key: Option<String>,
    pub label: String,
    pub detail: Option<String>,
    pub source: String,
    pub quota: Option<QuotaSnapshot>,
    /// When the account was last stored or refreshed.
    pub updated_at: Timestamp,
    /// The vendor refused the account's sign-in.
    pub signed_out: bool,
}

impl StoredAccount {
    /// The account's public metadata.
    pub fn meta(&self) -> AccountMeta {
        AccountMeta {
            id: self.id.as_str().to_owned(),
            label: self.label.clone(),
            detail: self.detail.clone(),
            updated_at: self.updated_at,
            source: self.source.clone(),
            identity_key: self.identity_key.clone(),
            signed_out: self.signed_out,
        }
    }
}

/// The storage of the registry's entries, which the program implements.
pub trait RegistryStore: Send + Sync {
    /// The entry's accounts in the list's order.
    fn accounts(&self, entry: ProviderId) -> BoxFuture<'_, Result<Vec<StoredAccount>, StoreError>>;

    /// The entry's account `account`; none when it holds no such account.
    fn account(&self, entry: ProviderId, account: CredentialId) -> BoxFuture<'_, Result<Option<StoredAccount>, StoreError>>;

    /// The entry's credential pool, which its subscription providers read
    /// and refresh their accounts' secrets through.
    fn pool(&self, entry: ProviderId) -> Arc<dyn CredentialPool>;

    /// Keeps `snapshot` as the account's quota.
    fn set_quota(
        &self,
        entry: ProviderId,
        account: CredentialId,
        snapshot: Arc<QuotaSnapshot>,
    ) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Tells whoever shows the entry that whether its accounts can be used
    /// changed.
    fn entry_changed(&self, entry: ProviderId) -> BoxFuture<'_, ()>;

    /// The entry's kept catalog.
    fn catalog(&self, entry: ProviderId) -> BoxFuture<'_, Result<Option<CatalogRecord>, StoreError>>;

    /// Keeps `record` as the entry's catalog.
    fn put_catalog(&self, entry: ProviderId, record: CatalogRecord) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Drops the entry's kept catalog.
    fn delete_catalog(&self, entry: ProviderId) -> BoxFuture<'_, Result<(), StoreError>>;
}
