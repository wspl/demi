//! The model catalog cache: one validated record per entry, held in memory and
//! in the program's store.
//!
//! - A record is fresh for 15 minutes.
//! - A refresh runs one at a time, in a task of the cache's own that a reader
//!   who stops waiting does not cancel.
//! - A failed refresh keeps the last good record and holds off automatic
//!   refreshes for a minute.
//! - A source answer marked stale, such as the models.dev copy returned after a
//!   failed request, counts as a failed refresh.
//! - A refresh that ends, with another catalog than the one held or with a
//!   failure, is announced to the cache's subscribers, so the pages that list
//!   the catalog hear of it.
//! - A page never waits for a source: [`ModelCatalogCache::known`] answers
//!   what the cache holds and starts the refresh a source with no record
//!   needs, whose end the pages hear of.
//!
//! The last rule is why a provider keeps no catalog cache of its own: the
//! backend would treat its cached answer as a failure, and one cache per
//! catalog is enough.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_shared_types::{CatalogRecord, Clock, ProviderModelList, Timestamp};
use demi_web_api_protocol::ids::ProviderId;
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use garde::Validate;
use jiff::SignedDuration;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::store::{RegistryStore, StoreError};

/// How long a record stays fresh.
const FRESH_FOR: SignedDuration = SignedDuration::from_mins(15);

/// How long a failed refresh holds off the next automatic one.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The longest a refresh may take.
const REFRESH_LIMIT: Duration = Duration::from_secs(10);

/// A read of a catalog's source.
pub type CatalogFetch = BoxFuture<'static, Result<ProviderModelList, String>>;

type Refresh = Shared<BoxFuture<'static, Result<Arc<CatalogRecord>, String>>>;

pub struct ModelCatalogCache {
    store: Arc<dyn RegistryStore>,
    clock: Arc<dyn Clock>,
    /// The edge's runtime, which refreshes run on, whoever asked.
    edge: tokio::runtime::Handle,
    refreshes: TaskTracker,
    /// Cancelled at shutdown; every entry's token is its child.
    closing: CancellationToken,
    /// A `std` mutex: readers on any thread look entries up and never await
    /// under it.
    entries: Mutex<HashMap<ProviderId, Arc<Entry>>>,
    /// Woken by each refresh that kept another catalog than the entry held.
    changes: watch::Sender<()>,
}

/// The cache of one entry under one key.
struct Entry {
    key: String,
    /// Cancelled when the entry is invalidated, its key changes or the cache
    /// closes; a cancelled refresh writes nothing back.
    cancel: CancellationToken,
    /// A `std` mutex, never held across an await.
    state: Mutex<EntryState>,
}

#[derive(Default)]
struct EntryState {
    /// The record once read from storage: `Some(None)` when storage holds
    /// none under this entry's key.
    record: Option<Option<Arc<CatalogRecord>>>,
    /// The source's revision the record was read under: the one a refresh
    /// started with, or, for a record loaded from storage, the one current
    /// when it was loaded.
    revision: Option<String>,
    failure: Option<Failure>,
    refresh: Option<Refresh>,
}

/// The last refresh failed.
struct Failure {
    message: String,
    /// Automatic refreshes wait until then.
    retry_at: Instant,
}

impl Entry {
    fn lock(&self) -> std::sync::MutexGuard<'_, EntryState> {
        // A section only reads or replaces fields, so a poisoned state is
        // whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ModelCatalogCache {
    pub fn new(
        store: Arc<dyn RegistryStore>,
        clock: Arc<dyn Clock>,
        edge: tokio::runtime::Handle,
    ) -> Self {
        Self {
            store,
            clock,
            edge,
            refreshes: TaskTracker::new(),
            closing: CancellationToken::new(),
            entries: Mutex::default(),
            changes: watch::Sender::new(()),
        }
    }

    /// Hears of each refresh that kept another catalog than the one its entry
    /// held; the receiver's `changed` ends once the cache is dropped.
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    /// The catalog of entry `provider` under `key`:
    ///
    /// - A fresh record comes at once.
    /// - An expired record, or one read under another `revision` of the source,
    ///   comes at once, marked stale, while one refresh starts or runs.
    /// - Without a record, or when `force` asks, the read returns what the
    ///   refresh returns.
    ///
    /// `fetch` reads the source when a refresh starts. A failed refresh returns
    /// the kept record marked stale with the failure as a warning, or, without
    /// a kept record, the failure.
    pub async fn read(
        &self,
        provider: &ProviderId,
        key: String,
        revision: Option<String>,
        fetch: impl FnOnce() -> CatalogFetch,
        force: bool,
    ) -> Result<ProviderModelList, String> {
        let answer = self.lookup(provider, key, revision, fetch, force, true).await?;
        Ok(answer.expect("a read that waits answers"))
    }

    /// What [`read`](Self::read) answers at once, never waiting for a source:
    /// none while the entry has no record to show and its source is being
    /// read, which a refresh the read starts if none runs; its end is
    /// announced to the subscribers. A source known to fail answers its failure
    /// until the next automatic refresh.
    pub async fn known(
        &self,
        provider: &ProviderId,
        key: String,
        revision: Option<String>,
        fetch: impl FnOnce() -> CatalogFetch,
    ) -> Result<Option<ProviderModelList>, String> {
        self.lookup(provider, key, revision, fetch, false, false).await
    }

    /// The one read behind both: the answer at once, or the refresh's when
    /// `wait` says to wait for it; none when it would wait and `wait` is false.
    async fn lookup(
        &self,
        provider: &ProviderId,
        key: String,
        revision: Option<String>,
        fetch: impl FnOnce() -> CatalogFetch,
        force: bool,
        wait: bool,
    ) -> Result<Option<ProviderModelList>, String> {
        if self.closing.is_cancelled() {
            return Err("The model catalog cache is closed".into());
        }
        let entry = self.entry(provider, key);
        let record = self.stored(provider, &entry, &revision).await?;
        let refresh = {
            let mut state = entry.lock();
            // A refresh may have replaced the record while it loaded.
            let record = state.record.clone().flatten().or(record);
            let now = self.clock.now();
            let current = state.revision == revision;
            if let (false, None, Some(record)) = (
                force,
                &state.failure,
                record
                    .as_ref()
                    .filter(|record| current && fresh(record, now)),
            ) {
                return Ok(Some(record.catalog.clone()));
            }
            let holding_off = state
                .failure
                .as_ref()
                .filter(|failure| Instant::now() < failure.retry_at)
                .map(|failure| failure.message.clone());
            if let (false, Some(message)) = (force, &holding_off) {
                return match record {
                    Some(record) => Ok(Some(stale(&record, message))),
                    None => Err(message.clone()),
                };
            }
            let refresh = match &state.refresh {
                Some(refresh) => refresh.clone(),
                None => {
                    // Taken before the source is read, so a revision that
                    // changes during the read leaves the record stale.
                    let refresh = self.start(provider, &entry, revision, fetch());
                    state.refresh = Some(refresh.clone());
                    refresh
                }
            };
            if let (false, Some(record)) = (force, &record) {
                let failure = state
                    .failure
                    .as_ref()
                    .map(|failure| failure.message.as_str());
                return Ok(Some(match failure {
                    Some(message) => stale(record, message),
                    None => stale_without_warning(record),
                }));
            }
            refresh
        };
        if !wait {
            return Ok(None);
        }
        match refresh.await {
            Ok(record) => Ok(Some(record.catalog.clone())),
            Err(message) => match entry.lock().record.clone().flatten() {
                Some(record) => Ok(Some(stale(&record, &message))),
                None => Err(message),
            },
        }
    }

    /// Drops the entry's record from memory and storage, and cancels its
    /// refresh, which writes nothing back.
    pub async fn invalidate(
        &self,
        provider: &ProviderId,
    ) -> Result<(), StoreError> {
        let entry = self.lock().remove(provider);
        if let Some(entry) = &entry {
            entry.cancel.cancel();
            let refresh = entry.lock().refresh.clone();
            if let Some(refresh) = refresh {
                // Its outcome belongs to the readers it cancels; waiting
                // only orders its last write before the deletion below.
                let _ = refresh.await;
            }
        }
        self.store.delete_catalog(provider.clone()).await
    }

    /// Cancels the running refreshes and waits for them.
    pub async fn close(&self) {
        self.closing.cancel();
        self.refreshes.close();
        self.refreshes.wait().await;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ProviderId, Arc<Entry>>> {
        // A section only looks up, inserts or removes an entry.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The entry under `key`; a changed key starts the entry afresh and
    /// cancels the refresh of the old one.
    fn entry(&self, provider: &ProviderId, key: String) -> Arc<Entry> {
        let mut entries = self.lock();
        if let Some(entry) = entries.get(provider).filter(|entry| entry.key == key) {
            return entry.clone();
        }
        let entry = Arc::new(Entry {
            key,
            cancel: self.closing.child_token(),
            state: Mutex::default(),
        });
        if let Some(previous) = entries.insert(provider.clone(), entry.clone()) {
            previous.cancel.cancel();
        }
        entry
    }

    /// The record storage holds under the entry's key, read once and taken
    /// as read under `revision`, the source's revision now. A stored record
    /// that fails validation is the entry's failure, never repaired or
    /// dropped.
    async fn stored(
        &self,
        provider: &ProviderId,
        entry: &Entry,
        revision: &Option<String>,
    ) -> Result<Option<Arc<CatalogRecord>>, String> {
        if let Some(record) = entry.lock().record.clone() {
            return Ok(record);
        }
        let stored = self
            .store
            .catalog(provider.clone())
            .await
            .map_err(|error| error.to_string())?;
        let record = stored
            .filter(|record| record.key == entry.key)
            .map(Arc::new);
        let mut state = entry.lock();
        // A concurrent read or a refresh may have set it meanwhile.
        if let Some(record) = &state.record {
            return Ok(record.clone());
        }
        state.revision = revision.clone();
        Ok(state.record.insert(record).clone())
    }

    /// Starts a refresh of the entry under the source's `revision` as a
    /// task of the cache.
    fn start(
        &self,
        provider: &ProviderId,
        entry: &Arc<Entry>,
        revision: Option<String>,
        fetch: CatalogFetch,
    ) -> Refresh {
        let task = refresh(
            self.store.clone(),
            self.clock.clone(),
            self.changes.clone(),
            provider.clone(),
            entry.clone(),
            revision,
            fetch,
        );
        let handle = self.refreshes.spawn_on(task, &self.edge);
        async move {
            handle
                .await
                .unwrap_or_else(|error| Err(format!("The catalog refresh failed: {error}")))
        }
        .boxed()
        .shared()
    }
}

/// Whether `record` was checked less than 15 minutes before `now`.
fn fresh(record: &CatalogRecord, now: Timestamp) -> bool {
    now.to_jiff().duration_since(record.checked_at.to_jiff()) < FRESH_FOR
}

/// The kept record, marked stale, with `failure` among its warnings.
fn stale(record: &CatalogRecord, failure: &str) -> ProviderModelList {
    let mut catalog = stale_without_warning(record);
    catalog.warnings.push(failure.to_owned());
    catalog
}

fn stale_without_warning(record: &CatalogRecord) -> ProviderModelList {
    ProviderModelList {
        stale: true,
        ..record.catalog.clone()
    }
}

/// One read of the entry's source within the limit, and, when it answers a
/// usable catalog, the new record, read under `revision`, in storage and in
/// memory. A cancelled refresh writes nothing.
async fn refresh(
    store: Arc<dyn RegistryStore>,
    clock: Arc<dyn Clock>,
    changes: watch::Sender<()>,
    provider: ProviderId,
    entry: Arc<Entry>,
    revision: Option<String>,
    fetch: CatalogFetch,
) -> Result<Arc<CatalogRecord>, String> {
    let answer = tokio::select! {
        () = entry.cancel.cancelled() => Err("The catalog refresh was cancelled".to_owned()),
        answer = tokio::time::timeout(REFRESH_LIMIT, fetch) => match answer {
            Err(_) => Err("Model catalog request timed out".to_owned()),
            Ok(answer) => answer.and_then(usable),
        },
    };
    let outcome = match answer {
        Ok(catalog) => {
            let record = CatalogRecord {
                key: entry.key.clone(),
                checked_at: clock.now(),
                catalog,
            };
            if entry.cancel.is_cancelled() {
                Err("The catalog refresh was cancelled".to_owned())
            } else {
                match store.put_catalog(provider, record.clone()).await {
                    Ok(()) => Ok(Arc::new(record)),
                    Err(error) => Err(format!("The catalog could not be stored: {error}")),
                }
            }
        }
        Err(message) => Err(message),
    };
    let mut state = entry.lock();
    state.refresh = None;
    if !entry.cancel.is_cancelled() {
        match &outcome {
            Ok(record) => {
                let held = state.record.clone().flatten();
                if held.is_none_or(|held| held.catalog != record.catalog) {
                    changes.send_replace(());
                }
                state.record = Some(Some(record.clone()));
                state.revision = revision;
                state.failure = None;
            }
            Err(message) => {
                state.failure = Some(Failure {
                    message: message.clone(),
                    retry_at: Instant::now() + RETRY_AFTER,
                });
                // The pages list the failure, as a warning on the kept record
                // or as the entry's reason for having no models.
                changes.send_replace(());
            }
        }
    }
    outcome
}

/// The catalog a source answered, when it can be kept: valid, and not a
/// stale copy of the source's own, which counts as a failed refresh.
fn usable(catalog: ProviderModelList) -> Result<ProviderModelList, String> {
    catalog.validate().map_err(|report| {
        format!(
            "The catalog cannot be read: {}",
            report.to_string().trim_end()
        )
    })?;
    if catalog.stale {
        return Err(if catalog.warnings.is_empty() {
            "The provider answered a stale model catalog".to_owned()
        } else {
            catalog.warnings.join("; ")
        });
    }
    Ok(catalog)
}
