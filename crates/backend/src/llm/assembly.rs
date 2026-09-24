//! Provider assembly (`providers.md` § Inference admission and runtime
//! ownership): the provider of each entry and account, built through the
//! entry's family from a fresh read of the entry and reused only while the
//! entry read is unchanged, so a lookup that completes after an edit cannot
//! keep later requests on a stale configuration.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use demi_core::Clock;
use demi_provider::credentials::CredentialPool;
use demi_provider::{Provider, ProviderRuntime};
use demi_provider_claude_code::Placement;
use demi_web_api::ids::{CredentialId, ProviderId};

use super::catalog_cache::ModelCatalogCache;
use super::families::{
    AccountBinding, ApiKeyArgs, FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily,
    SubscriptionArgs,
};
use super::vendors::VendorCatalog;
use crate::storage::StorageError;
use crate::vault::entries::{EntryCredential, ProviderEntry, Vault};
use crate::vault::quotas::AccountQuotas;

/// Why an entry's provider could not be built.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AssemblyError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The entry names a family this backend does not register.
    #[error("the provider family {0} is not available")]
    UnknownFamily(String),
    #[error(transparent)]
    Family(#[from] FamilyError),
    /// The entry's provider says it runs a process, but its family builds
    /// no runtime over a placement.
    #[error("the provider family {0} cannot run its process")]
    NoProcessRuntime(String),
}

/// An entry's provider for its active account, with the entry read it was
/// built from.
type Built = (ProviderEntry, Arc<dyn Provider>);

/// The providers of every entry, shared by the edge and the shards.
pub(crate) struct ProviderAssembly {
    vault: Vault,
    families: FamilyRegistry,
    quotas: Arc<AccountQuotas>,
    catalogs: ModelCatalogCache,
    vendors: VendorCatalog,
    /// The shared services' HTTP client, which providers make their own
    /// requests with.
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    /// The provider of each entry's active account, with the entry it was
    /// built from. A `std` mutex: lookups never await under it.
    built: Mutex<HashMap<ProviderId, Built>>,
}

impl ProviderAssembly {
    pub(crate) fn new(
        vault: Vault,
        families: FamilyRegistry,
        quotas: Arc<AccountQuotas>,
        catalogs: ModelCatalogCache,
        vendors: VendorCatalog,
        http: reqwest::Client,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            vault,
            families,
            quotas,
            catalogs,
            vendors,
            http,
            clock,
            built: Mutex::default(),
        }
    }

    pub(crate) fn vault(&self) -> &Vault {
        &self.vault
    }

    pub(crate) fn families(&self) -> &FamilyRegistry {
        &self.families
    }

    pub(crate) fn quotas(&self) -> &Arc<AccountQuotas> {
        &self.quotas
    }

    pub(crate) fn catalogs(&self) -> &ModelCatalogCache {
        &self.catalogs
    }

    pub(crate) fn vendors(&self) -> &VendorCatalog {
        &self.vendors
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ProviderId, Built>> {
        // A section only looks up or replaces one entry's provider.
        self.built.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The provider of `entry`, a fresh read of the entry, for its active
    /// account: the one built before while the entry is unchanged, otherwise
    /// a new one.
    pub(crate) async fn provider_for(&self, entry: &ProviderEntry) -> Result<Arc<dyn Provider>, AssemblyError> {
        let reused = self
            .lock()
            .get(&entry.id)
            .filter(|(built_from, _)| built_from == entry)
            .map(|(_, provider)| provider.clone());
        if let Some(provider) = reused {
            return Ok(provider);
        }
        let provider = self.build(entry, entry.active()).await?;
        // Two builds for one entry race only to insert the same provider.
        self.lock().insert(entry.id.clone(), (entry.clone(), provider.clone()));
        Ok(provider)
    }

    /// The provider of `entry` for its account `account`, whichever is
    /// active; `None` when the entry holds no such account.
    pub(crate) async fn for_account(
        &self,
        entry: &ProviderEntry,
        account: &CredentialId,
    ) -> Result<Option<Arc<dyn Provider>>, AssemblyError> {
        if entry.active() == Some(account) {
            return self.provider_for(entry).await.map(Some);
        }
        let stored = self.vault.account(entry.id.clone(), account.clone()).await?;
        if stored.is_none() {
            return Ok(None);
        }
        self.build(entry, Some(account)).await.map(Some)
    }

    /// A provider of `family` over `pool` with no account yet, for a login
    /// whose entry does not exist until the login completes.
    pub(crate) fn detached(
        &self,
        family: &str,
        id: &str,
        label: &str,
        pool: Arc<dyn CredentialPool>,
    ) -> Result<Arc<dyn Provider>, AssemblyError> {
        let registered = self.family(family)?;
        let credential = FamilyCredential::Subscription(SubscriptionArgs { pool, account: None });
        Ok(registered.provider(self.args(id.to_owned(), label.to_owned(), credential))?)
    }

    /// Whether `entry`'s provider runs a process on a Host, which then is the
    /// user's Cloud (`claude-code.md` § Where it runs).
    pub(crate) async fn runs_a_process(&self, entry: &ProviderEntry) -> Result<bool, AssemblyError> {
        Ok(self.provider_for(entry).await?.capabilities().process_host)
    }

    /// A session's runtime of `entry`'s provider for `account`, whose
    /// process `placement` starts, for a provider that runs one
    /// (`claude-code.md` § How a runtime gets its process).
    pub(crate) async fn process_runtime(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
        placement: Rc<dyn Placement>,
    ) -> Result<Box<dyn ProviderRuntime>, AssemblyError> {
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, account).await?;
        let runtime = family
            .process_runtime(args, placement)
            .ok_or_else(|| AssemblyError::NoProcessRuntime(entry.family.clone()))?;
        Ok(runtime?)
    }

    /// Forgets the entry's provider and catalog after its configuration or
    /// active account changed.
    pub(crate) async fn invalidate(&self, id: &ProviderId) -> Result<(), StorageError> {
        self.lock().remove(id);
        self.catalogs.invalidate(id).await
    }

    /// Forgets everything held of a deleted entry.
    pub(crate) async fn forget(&self, id: &ProviderId) -> Result<(), StorageError> {
        self.quotas.forget_entry(id);
        self.invalidate(id).await
    }

    /// Stops the catalog refreshes and waits for the quota writes.
    pub(crate) async fn close(&self) {
        self.catalogs.close().await;
        self.quotas.close().await;
    }

    pub(crate) fn family(&self, name: &str) -> Result<&Arc<dyn ProviderFamily>, AssemblyError> {
        self.families
            .get(name)
            .ok_or_else(|| AssemblyError::UnknownFamily(name.to_owned()))
    }

    async fn build(
        &self,
        entry: &ProviderEntry,
        account: Option<&CredentialId>,
    ) -> Result<Arc<dyn Provider>, AssemblyError> {
        let family = self.family(&entry.family)?;
        let args = self.entry_args(entry, account).await?;
        Ok(family.provider(args)?)
    }

    /// What `entry`'s family builds from for `account`: the entry's settings,
    /// or its pool with the account and its quota store.
    async fn entry_args(&self, entry: &ProviderEntry, account: Option<&CredentialId>) -> Result<FamilyArgs, AssemblyError> {
        let credential = match &entry.credential {
            EntryCredential::ApiKey(config) => FamilyCredential::ApiKey(ApiKeyArgs {
                api_key: config.api_key.clone(),
                base_url: config.base_url.as_ref().map(|endpoint| endpoint.url().clone()),
                wire_api: config.wire_api,
                vendor: self.vendors.policy(config.vendor_id.as_deref()),
            }),
            EntryCredential::Subscription { .. } => {
                let binding = match account {
                    Some(account) => self
                        .vault
                        .account(entry.id.clone(), account.clone())
                        .await?
                        .map(|record| AccountBinding {
                            credential_id: record.id.as_str().to_owned(),
                            quota: self.quotas.store(&entry.id, &record),
                        }),
                    None => None,
                };
                FamilyCredential::Subscription(SubscriptionArgs {
                    pool: self.vault.pool(entry.id.clone()),
                    account: binding,
                })
            }
        };
        Ok(self.args(entry.id.as_str().to_owned(), entry.label.clone(), credential))
    }

    fn args(&self, entry_id: String, label: String, credential: FamilyCredential) -> FamilyArgs {
        FamilyArgs {
            entry_id,
            label,
            credential,
            http: self.http.clone(),
            clock: self.clock.clone(),
            models_dev: self.vendors.models_dev().clone(),
        }
    }
}
