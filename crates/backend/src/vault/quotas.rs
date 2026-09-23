//! Each subscription account's quota snapshot (`usage-and-quota.md` § One
//! snapshot per account): held in memory by the process that uses the
//! account and written through to the account's record, so that it outlives
//! a rebuilt provider and a restart. A write that fails is logged; it costs
//! at most a later probe.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use demi_core::QuotaSnapshot;
use demi_provider::quota::QuotaSnapshotStore;
use demi_web_api::ids::{CredentialId, ProviderId};
use tokio_util::task::TaskTracker;

use crate::storage::control::ControlService;
use crate::storage::providers::CredentialRow;

type Account = (ProviderId, CredentialId);

/// The snapshots of every account this process uses.
pub(crate) struct AccountQuotas {
    control: ControlService,
    /// The edge's runtime, which outlives the shards: the record writes run
    /// there, wherever the update came from.
    edge: tokio::runtime::Handle,
    writes: TaskTracker,
    /// A `std` mutex: probes at the edge and observations on the shards
    /// update snapshots from several threads, and no section awaits. An
    /// account is held from its first use; `None` holds no snapshot yet.
    held: Mutex<HashMap<Account, Option<Arc<QuotaSnapshot>>>>,
}

impl AccountQuotas {
    pub(crate) fn new(control: ControlService, edge: tokio::runtime::Handle) -> Arc<Self> {
        Arc::new(Self {
            control,
            edge,
            writes: TaskTracker::new(),
            held: Mutex::default(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Account, Option<Arc<QuotaSnapshot>>>> {
        // A section only reads or replaces one entry, so a poisoned map is
        // whole.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The store of the account `record`, which starts from the record's
    /// snapshot the first time this process uses the account.
    pub(crate) fn store(
        self: &Arc<Self>,
        provider: &ProviderId,
        record: &CredentialRow,
    ) -> Arc<dyn QuotaSnapshotStore> {
        let account = (provider.clone(), record.id.clone());
        self.lock()
            .entry(account.clone())
            .or_insert_with(|| record.quota.clone().map(Arc::new));
        Arc::new(AccountQuota {
            quotas: self.clone(),
            account,
        })
    }

    /// The account's snapshot now: this process's newest, else its record's.
    pub(crate) fn latest(&self, provider: &ProviderId, record: &CredentialRow) -> Option<QuotaSnapshot> {
        let account = (provider.clone(), record.id.clone());
        match self.lock().get(&account) {
            Some(held) => held.as_deref().cloned(),
            None => record.quota.clone(),
        }
    }

    /// Forgets a removed account's snapshot.
    pub(crate) fn forget_account(&self, provider: &ProviderId, account: &CredentialId) {
        self.lock().remove(&(provider.clone(), account.clone()));
    }

    /// Forgets the snapshots of a deleted entry's accounts.
    pub(crate) fn forget_entry(&self, provider: &ProviderId) {
        self.lock().retain(|(held, _), _| held != provider);
    }

    /// Waits for the record writes still running.
    pub(crate) async fn close(&self) {
        self.writes.close();
        self.writes.wait().await;
    }

    /// Writes the account's newest snapshot to its record. A write stores
    /// whatever is newest when it runs, so writes that run out of order
    /// still leave the newest snapshot stored.
    fn write_through(self: &Arc<Self>, account: Account) {
        let quotas = self.clone();
        self.writes.spawn_on(
            async move {
                let newest = quotas.lock().get(&account).cloned().flatten();
                let Some(snapshot) = newest else {
                    return;
                };
                let (provider, id) = account;
                if let Err(error) = quotas.control.set_credential_quota(provider, id, snapshot).await {
                    tracing::warn!(
                        error = &error as &dyn std::error::Error,
                        "an account's quota snapshot was not stored; the next probe reads it again"
                    );
                }
            },
            &self.edge,
        );
    }
}

/// One account's snapshot store.
struct AccountQuota {
    quotas: Arc<AccountQuotas>,
    account: Account,
}

impl QuotaSnapshotStore for AccountQuota {
    fn latest(&self) -> Option<Arc<QuotaSnapshot>> {
        self.quotas.lock().get(&self.account).cloned().flatten()
    }

    fn update(&self, next: &mut dyn FnMut(Option<&QuotaSnapshot>) -> QuotaSnapshot) -> Arc<QuotaSnapshot> {
        let snapshot = {
            let mut held = self.quotas.lock();
            let kept = held.entry(self.account.clone()).or_default();
            let snapshot = Arc::new(next(kept.as_deref()));
            *kept = Some(snapshot.clone());
            snapshot
        };
        self.quotas.write_through(self.account.clone());
        snapshot
    }
}
