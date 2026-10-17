//! Each subscription account's quota snapshot (`usage-and-quota.md` § One
//! snapshot per account): held in memory by the process that uses the
//! account and written through to the account's record, so that it outlives
//! a rebuilt provider and a restart. A write that fails is logged; it costs
//! at most a later probe. Whether an account can be used is what routes a
//! request, so the pages are told when it changes: a snapshot written, an
//! account marked used up, and the moment either lets the account be used
//! again (`images.md` § Which entry makes an image).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use demi_backend_database::providers::CredentialRow;
use demi_provider_common::quota::QuotaSnapshotStore;
use demi_shared_types::{Clock, QuotaSnapshot, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::entries::Vault;

type Account = (ProviderId, CredentialId);

/// The snapshots of every account this process uses.
pub struct AccountQuotas {
    /// Where the snapshots are stored, and whose pages see them change.
    vault: Vault,
    /// The edge's runtime, which outlives the shards: the record writes run
    /// there, wherever the update came from.
    edge: tokio::runtime::Handle,
    writes: TaskTracker,
    /// A `std` mutex: probes at the edge and observations on the shards
    /// update snapshots from several threads, and no section awaits. An
    /// account is held from its first use; `None` holds no snapshot yet.
    held: Mutex<HashMap<Account, Option<Arc<QuotaSnapshot>>>>,
    clock: Arc<dyn Clock>,
    /// The moments an entry's accounts can be used again that a timer waits
    /// for, each once.
    awaited: Mutex<HashSet<(ProviderId, Timestamp)>>,
    /// Ends the timers when the backend closes.
    stop: CancellationToken,
}

impl AccountQuotas {
    pub fn new(vault: Vault, edge: tokio::runtime::Handle, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            vault,
            edge,
            writes: TaskTracker::new(),
            held: Mutex::default(),
            clock,
            awaited: Mutex::default(),
            stop: CancellationToken::new(),
        })
    }

    /// Tells the pages of entry `provider`'s users that an account of it
    /// was marked used up until `until`, and again at `until`, when it can
    /// be used again.
    pub fn used_up(self: &Arc<Self>, provider: &ProviderId, until: Timestamp) {
        let quotas = self.clone();
        let entry = provider.clone();
        self.writes.spawn_on(async move { quotas.vault.mark_entry_changed(&entry).await }, &self.edge);
        self.announce_at(provider, until);
    }

    /// Tells the pages of entry `provider`'s users at `moment`, once for
    /// each moment, unless the backend closes first.
    fn announce_at(self: &Arc<Self>, provider: &ProviderId, moment: Timestamp) {
        let key = (provider.clone(), moment);
        if !self.awaited.lock().unwrap_or_else(PoisonError::into_inner).insert(key.clone()) {
            return;
        }
        let wait = moment.to_jiff().duration_since(self.clock.now().to_jiff());
        let wait = std::time::Duration::try_from(wait).unwrap_or_default();
        let quotas = self.clone();
        self.writes.spawn_on(
            async move {
                tokio::select! {
                    () = quotas.stop.cancelled() => {}
                    () = tokio::time::sleep(wait) => quotas.vault.mark_entry_changed(&key.0).await,
                }
                quotas.awaited.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
            },
            &self.edge,
        );
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Account, Option<Arc<QuotaSnapshot>>>> {
        // A section only reads or replaces one entry, so a poisoned map is
        // whole.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The store of the account `record`, which starts from the record's
    /// snapshot the first time this process uses the account.
    pub fn store(
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
    pub fn latest(&self, provider: &ProviderId, record: &CredentialRow) -> Option<QuotaSnapshot> {
        let account = (provider.clone(), record.id.clone());
        match self.lock().get(&account) {
            Some(held) => held.as_deref().cloned(),
            None => record.quota.clone(),
        }
    }

    /// The edge's runtime, where a request with a provider's own HTTP
    /// client runs (`concurrency.md` § Programs and threads).
    pub fn edge(&self) -> &tokio::runtime::Handle {
        &self.edge
    }

    /// Forgets a removed account's snapshot.
    pub fn forget_account(&self, provider: &ProviderId, account: &CredentialId) {
        self.lock().remove(&(provider.clone(), account.clone()));
    }

    /// Forgets the snapshots of a deleted entry's accounts.
    pub fn forget_entry(&self, provider: &ProviderId) {
        self.lock().retain(|(held, _), _| held != provider);
    }

    /// Ends the timers and waits for the record writes still running.
    pub async fn close(&self) {
        self.stop.cancel();
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
                // A window used up routes requests elsewhere until it
                // resets, which the pages are told then.
                let now = quotas.clock.now();
                let resets = snapshot
                    .windows
                    .iter()
                    .filter(|window| window.used_percent.is_some_and(|used| used >= 100.0))
                    .filter_map(|window| window.resets_at)
                    .filter(|reset| *reset > now)
                    .max();
                let (provider, id) = account;
                let stored = quotas
                    .vault
                    .control()
                    .set_credential_quota(provider.clone(), id, snapshot)
                    .await;
                if let Err(error) = stored {
                    tracing::warn!(
                        error = &error as &dyn std::error::Error,
                        "an account's quota snapshot was not stored; the next probe reads it again"
                    );
                    return;
                }
                quotas.vault.mark_entry_changed(&provider).await;
                if let Some(reset) = resets {
                    quotas.announce_at(&provider, reset);
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

    fn update(
        &self,
        next: &mut dyn FnMut(Option<&QuotaSnapshot>) -> QuotaSnapshot,
    ) -> Arc<QuotaSnapshot> {
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
