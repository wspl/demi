//! The model catalog cache (`models.md` § Catalog cache): one validated
//! record per entry, held in memory and in the control store's
//! `model_catalogs` table, fresh for 15 minutes, refreshed once at a time by
//! a task of the cache's own that a reader who stops waiting does not
//! cancel. A failed refresh keeps the last good record and holds off
//! automatic refreshes for a minute.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use demi_core::{Clock, ProviderModelList, Timestamp};
use demi_web_api::ids::ProviderId;
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use garde::Validate;
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::storage::control::ControlService;

/// How long a record stays fresh.
const FRESH_FOR: SignedDuration = SignedDuration::from_mins(15);

/// How long a failed refresh holds off the next automatic one.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The longest a refresh may take.
const REFRESH_LIMIT: Duration = Duration::from_secs(10);

/// An entry's cached catalog as the control store keeps it: model metadata
/// only, under the key of the configuration and account it was read for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CatalogRecord {
    /// The digest of the entry's configuration and active account.
    #[garde(length(min = 1))]
    pub(crate) key: String,
    /// When the source last answered.
    #[garde(skip)]
    pub(crate) checked_at: Timestamp,
    #[garde(dive)]
    pub(crate) catalog: ProviderModelList,
}

/// A read of a catalog's source.
pub(crate) type CatalogFetch = BoxFuture<'static, Result<ProviderModelList, String>>;

type Refresh = Shared<BoxFuture<'static, Result<Arc<CatalogRecord>, String>>>;

pub(crate) struct ModelCatalogCache {
    control: ControlService,
    clock: Arc<dyn Clock>,
    /// The edge's runtime, which refreshes run on, whoever asked.
    edge: tokio::runtime::Handle,
    refreshes: TaskTracker,
    /// Cancelled at shutdown; every entry's token is its child.
    closing: CancellationToken,
    /// A `std` mutex: readers on any thread look entries up and never await
    /// under it.
    entries: Mutex<HashMap<ProviderId, Arc<Entry>>>,
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
    pub(crate) fn new(control: ControlService, clock: Arc<dyn Clock>, edge: tokio::runtime::Handle) -> Self {
        Self {
            control,
            clock,
            edge,
            refreshes: TaskTracker::new(),
            closing: CancellationToken::new(),
            entries: Mutex::default(),
        }
    }

    /// The catalog of entry `provider` under `key` (`models.md` § Freshness
    /// and refresh): a fresh record at once; an expired one at once, marked
    /// stale, while one refresh starts or runs; without a record, or when
    /// `force` asks, what the refresh returns. `fetch` reads the source when
    /// a refresh starts. A failed refresh returns the kept record marked
    /// stale with the failure as a warning, or, without one, the failure.
    pub(crate) async fn read(
        &self,
        provider: &ProviderId,
        key: String,
        fetch: impl FnOnce() -> CatalogFetch,
        force: bool,
    ) -> Result<ProviderModelList, String> {
        if self.closing.is_cancelled() {
            return Err("The model catalog cache is closed".into());
        }
        let entry = self.entry(provider, key);
        let record = self.stored(provider, &entry).await?;
        let refresh = {
            let mut state = entry.lock();
            // A refresh may have replaced the record while it loaded.
            let record = state.record.clone().flatten().or(record);
            let now = self.clock.now();
            if let (false, None, Some(record)) = (
                force,
                &state.failure,
                record.as_ref().filter(|record| fresh(record, now)),
            ) {
                return Ok(record.catalog.clone());
            }
            let holding_off = state
                .failure
                .as_ref()
                .filter(|failure| Instant::now() < failure.retry_at)
                .map(|failure| failure.message.clone());
            if let (false, Some(message)) = (force, &holding_off) {
                return match record {
                    Some(record) => Ok(stale(&record, message)),
                    None => Err(message.clone()),
                };
            }
            let refresh = match &state.refresh {
                Some(refresh) => refresh.clone(),
                None => {
                    let refresh = self.start(provider, &entry, fetch());
                    state.refresh = Some(refresh.clone());
                    refresh
                }
            };
            if let (false, Some(record)) = (force, &record) {
                let failure = state.failure.as_ref().map(|failure| failure.message.as_str());
                return Ok(match failure {
                    Some(message) => stale(record, message),
                    None => stale_without_warning(record),
                });
            }
            refresh
        };
        match refresh.await {
            Ok(record) => Ok(record.catalog.clone()),
            Err(message) => match entry.lock().record.clone().flatten() {
                Some(record) => Ok(stale(&record, &message)),
                None => Err(message),
            },
        }
    }

    /// Drops the entry's record from memory and storage, and cancels its
    /// refresh, which writes nothing back.
    pub(crate) async fn invalidate(&self, provider: &ProviderId) -> Result<(), crate::storage::StorageError> {
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
        self.control.delete_catalog_record(provider.clone()).await
    }

    /// Cancels the running refreshes and waits for them.
    pub(crate) async fn close(&self) {
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

    /// The record storage holds under the entry's key, read once. A stored
    /// record that fails validation is the entry's failure, never repaired
    /// or dropped.
    async fn stored(&self, provider: &ProviderId, entry: &Entry) -> Result<Option<Arc<CatalogRecord>>, String> {
        if let Some(record) = entry.lock().record.clone() {
            return Ok(record);
        }
        let stored = self
            .control
            .catalog_record(provider.clone())
            .await
            .map_err(|error| error.to_string())?;
        let record = stored.filter(|record| record.key == entry.key).map(Arc::new);
        let mut state = entry.lock();
        // A concurrent read or a refresh may have set it meanwhile.
        Ok(state.record.get_or_insert(record).clone())
    }

    /// Starts a refresh of the entry as a task of the cache.
    fn start(&self, provider: &ProviderId, entry: &Arc<Entry>, fetch: CatalogFetch) -> Refresh {
        let task = refresh(
            self.control.clone(),
            self.clock.clone(),
            provider.clone(),
            entry.clone(),
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
/// usable catalog, the new record in storage and in memory. A cancelled
/// refresh writes nothing.
async fn refresh(
    control: ControlService,
    clock: Arc<dyn Clock>,
    provider: ProviderId,
    entry: Arc<Entry>,
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
                match control.put_catalog_record(provider, record.clone()).await {
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
                state.record = Some(Some(record.clone()));
                state.failure = None;
            }
            Err(message) => {
                state.failure = Some(Failure {
                    message: message.clone(),
                    retry_at: Instant::now() + RETRY_AFTER,
                });
            }
        }
    }
    outcome
}

/// The catalog a source answered, when it can be kept: valid, and not a
/// stale copy of the source's own, which counts as a failed refresh.
fn usable(catalog: ProviderModelList) -> Result<ProviderModelList, String> {
    catalog
        .validate()
        .map_err(|report| format!("The catalog cannot be read: {}", report.to_string().trim_end()))?;
    if catalog.stale {
        return Err(if catalog.warnings.is_empty() {
            "The provider answered a stale model catalog".to_owned()
        } else {
            catalog.warnings.join("; ")
        });
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use demi_core::ProviderModel;
    use demi_web_api::providers::CredentialKind;
    use futures_util::future::join_all;
    use tokio::sync::oneshot;

    use super::*;
    use crate::storage::control::testing;
    use crate::storage::providers::NewProvider;

    /// Wall time that follows Tokio's clock, so a paused test moves both.
    struct TokioClock {
        start: Timestamp,
        origin: Instant,
    }

    impl Clock for TokioClock {
        fn now(&self) -> Timestamp {
            let elapsed = i64::try_from(self.origin.elapsed().as_millis()).unwrap();
            Timestamp::from_millisecond(self.start.as_millisecond() + elapsed).unwrap()
        }
    }

    fn catalog(name: &str) -> ProviderModelList {
        ProviderModelList {
            models: vec![ProviderModel {
                id: "model".into(),
                display_name: name.into(),
                description: None,
                context_window: Some(1000),
                output_limit: None,
                supports_tools: Some(true),
                supports_attachments: Some(false),
                supports_video: None,
                accepted_extensions: None,
                supports_reasoning: Some(false),
                supported_thinking_efforts: None,
                default_thinking_effort: None,
                can_disable_thinking: None,
                service_tiers: Vec::new(),
                default_service_tier_id: None,
                cost: None,
            }],
            default_model_id: None,
            warnings: Vec::new(),
            source_fetched_at: "2026-09-13T00:00:00.000Z".parse().unwrap(),
            stale: false,
        }
    }

    struct Fixture {
        _data: tempfile::TempDir,
        control: ControlService,
        clock: Arc<TokioClock>,
        provider: ProviderId,
    }

    impl Fixture {
        async fn new() -> Self {
            let data = tempfile::tempdir().unwrap();
            let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
                .await
                .unwrap();
            let owner = testing::master(&control).await.id;
            let provider = ProviderId::try_from("provider").unwrap();
            let entry = NewProvider {
                id: provider.clone(),
                owner,
                family: "scripted".into(),
                kind: CredentialKind::ApiKey,
                label: "Scripted".into(),
                config: Some(b"sealed".to_vec()),
                active: None,
            };
            control.insert_provider(entry, Vec::new()).await.unwrap();
            let clock = Arc::new(TokioClock {
                start: "2026-09-24T09:50:00.000Z".parse().unwrap(),
                origin: Instant::now(),
            });
            Self {
                _data: data,
                control,
                clock,
                provider,
            }
        }

        fn cache(&self) -> ModelCatalogCache {
            ModelCatalogCache::new(
                self.control.clone(),
                self.clock.clone(),
                tokio::runtime::Handle::current(),
            )
        }
    }

    /// A source that answers `answer` and counts its reads.
    fn answering(
        reads: &Arc<AtomicUsize>,
        answer: Result<ProviderModelList, String>,
    ) -> impl FnOnce() -> CatalogFetch + Send + use<> {
        let reads = reads.clone();
        move || {
            reads.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(answer))
        }
    }

    /// A source that answers once `answer` is sent, and says when it
    /// started and when it was dropped before answering.
    struct Awaited {
        answer: oneshot::Receiver<ProviderModelList>,
        started: Arc<tokio::sync::Notify>,
        dropped: Arc<AtomicBool>,
    }

    fn awaiting(reads: &Arc<AtomicUsize>, awaited: Awaited) -> impl FnOnce() -> CatalogFetch + Send + use<> {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let reads = reads.clone();
        move || {
            reads.fetch_add(1, Ordering::SeqCst);
            let Awaited {
                answer,
                started,
                dropped,
            } = awaited;
            let dropped = Dropped(dropped);
            Box::pin(async move {
                let _dropped = dropped;
                started.notify_one();
                answer.await.map_err(|_| "no answer".to_owned())
            })
        }
    }

    fn source() -> (oneshot::Sender<ProviderModelList>, Awaited) {
        let (answer, answered) = oneshot::channel();
        let awaited = Awaited {
            answer: answered,
            started: Arc::new(tokio::sync::Notify::new()),
            dropped: Arc::new(AtomicBool::new(false)),
        };
        (answer, awaited)
    }

    fn name(list: &ProviderModelList) -> &str {
        &list.models[0].display_name
    }

    #[tokio::test(start_paused = true)]
    async fn a_fresh_record_serves_from_memory_and_storage_and_an_expired_one_refreshes_once_behind_its_readers() {
        let fixture = Fixture::new().await;
        let reads = Arc::new(AtomicUsize::new(0));
        let cache = fixture.cache();
        let cold = join_all((0..8).map(|_| {
            cache.read(
                &fixture.provider,
                "key".into(),
                answering(&reads, Ok(catalog("First"))),
                false,
            )
        }))
        .await;
        assert!(
            cold.iter()
                .all(|list| list.as_ref().is_ok_and(|list| name(list) == "First" && !list.stale))
        );
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&reads, Ok(catalog("Other"))),
                false,
            )
            .await
            .unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        cache.close().await;

        // A restart loads the stored record before it asks the source.
        let cache = fixture.cache();
        let restored = cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&reads, Ok(catalog("Other"))),
                false,
            )
            .await
            .unwrap();
        assert_eq!((name(&restored), reads.load(Ordering::SeqCst)), ("First", 1));

        // Expired: every reader gets the record at once, marked stale, and one
        // refresh starts; a forced read joins it.
        tokio::time::advance(Duration::from_secs(15 * 60)).await;
        let (answer, awaited) = source();
        let mut fetch = Some(awaiting(&reads, awaited));
        let mut stale = Vec::new();
        for _ in 0..8 {
            let source = fetch.take().map_or_else(
                || Box::new(answering(&reads, Ok(catalog("Never")))) as Box<dyn FnOnce() -> CatalogFetch>,
                |fetch| Box::new(fetch) as Box<dyn FnOnce() -> CatalogFetch>,
            );
            stale.push(
                cache
                    .read(&fixture.provider, "key".into(), source, false)
                    .await
                    .unwrap(),
            );
        }
        assert!(stale.iter().all(|list| list.stale && name(list) == "First"));
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        let forced = cache.read(
            &fixture.provider,
            "key".into(),
            answering(&reads, Ok(catalog("Never"))),
            true,
        );
        answer.send(catalog("Updated")).unwrap();
        let forced = forced.await.unwrap();
        assert_eq!(
            (name(&forced), forced.stale, reads.load(Ordering::SeqCst)),
            ("Updated", false, 2)
        );
        let stored = fixture
            .control
            .catalog_record(fixture.provider.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (stored.checked_at, name(&stored.catalog)),
            (fixture.clock.now(), "Updated")
        );
        cache.close().await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_refresh_keeps_the_record_holds_off_a_minute_and_a_forced_refresh_does_not_wait() {
        let fixture = Fixture::new().await;
        let reads = Arc::new(AtomicUsize::new(0));
        let failures = Arc::new(AtomicUsize::new(0));
        let cache = fixture.cache();
        cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&reads, Ok(catalog("First"))),
                false,
            )
            .await
            .unwrap();
        let kept = fixture.control.catalog_record(fixture.provider.clone()).await.unwrap();
        tokio::time::advance(Duration::from_secs(15 * 60)).await;
        let failed = cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&failures, Err("offline".into())),
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            (failed.stale, name(&failed), failed.warnings.clone()),
            (true, "First", vec!["offline".to_owned()])
        );
        assert_eq!(
            fixture.control.catalog_record(fixture.provider.clone()).await.unwrap(),
            kept
        );
        // Held off: no automatic refresh for a minute.
        let held = cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&failures, Err("offline".into())),
                false,
            )
            .await
            .unwrap();
        assert!(held.stale);
        assert_eq!(failures.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(59)).await;
        cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&failures, Err("offline".into())),
                true,
            )
            .await
            .unwrap();
        assert_eq!(failures.load(Ordering::SeqCst), 2);
        let recovered = cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&reads, Ok(catalog("Recovered"))),
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            (name(&recovered), recovered.stale, recovered.warnings.len()),
            ("Recovered", false, 0)
        );
        cache.close().await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_changed_key_or_an_invalidation_cancels_the_refresh_and_its_answer_writes_nothing() {
        let fixture = Fixture::new().await;
        let reads = Arc::new(AtomicUsize::new(0));
        let cache = Arc::new(fixture.cache());
        let (old_answer, awaited) = source();
        let (started, dropped) = (awaited.started.clone(), awaited.dropped.clone());
        let old = {
            let cache = cache.clone();
            let provider = fixture.provider.clone();
            let fetch = awaiting(&reads, awaited);
            tokio::spawn(async move { cache.read(&provider, "old-account".into(), fetch, false).await })
        };
        started.notified().await;
        cache.invalidate(&fixture.provider).await.unwrap();
        assert!(dropped.load(Ordering::SeqCst));
        assert!(old.await.unwrap().is_err());
        assert!(old_answer.send(catalog("Old account")).is_err());

        let new = cache
            .read(
                &fixture.provider,
                "new-account".into(),
                answering(&reads, Ok(catalog("New account"))),
                false,
            )
            .await
            .unwrap();
        assert_eq!(name(&new), "New account");
        // A changed key starts afresh and never serves the other key's record.
        let changed = cache
            .read(
                &fixture.provider,
                "changed-config".into(),
                answering(&reads, Ok(catalog("Changed"))),
                false,
            )
            .await
            .unwrap();
        assert_eq!((name(&changed), reads.load(Ordering::SeqCst)), ("Changed", 3));
        let stored = fixture
            .control
            .catalog_record(fixture.provider.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (stored.key.as_str(), name(&stored.catalog)),
            ("changed-config", "Changed")
        );
        fixture.control.delete_provider(fixture.provider.clone()).await.unwrap();
        assert_eq!(
            fixture.control.catalog_record(fixture.provider.clone()).await.unwrap(),
            None
        );
        cache.close().await;
    }

    #[tokio::test(start_paused = true)]
    async fn cold_failures_and_unusable_answers_are_explicit_a_refresh_times_out_and_closing_ends_its_readers() {
        let fixture = Fixture::new().await;
        let reads = Arc::new(AtomicUsize::new(0));
        let cache = Arc::new(fixture.cache());
        let offline = cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&reads, Err("offline".into())),
                true,
            )
            .await;
        assert_eq!(offline, Err("offline".into()));
        let mut invalid = catalog("Invalid");
        invalid.models[0].output_limit = Some(0);
        let refused = cache
            .read(&fixture.provider, "key".into(), answering(&reads, Ok(invalid)), true)
            .await;
        assert!(refused.unwrap_err().contains("cannot be read"));
        let mut stale = catalog("Stale");
        stale.stale = true;
        stale.warnings = vec!["Using stale models.dev catalog: offline".into()];
        let refused = cache
            .read(&fixture.provider, "key".into(), answering(&reads, Ok(stale)), true)
            .await;
        assert_eq!(refused, Err("Using stale models.dev catalog: offline".into()));
        assert_eq!(
            fixture.control.catalog_record(fixture.provider.clone()).await.unwrap(),
            None
        );

        // A source that never answers is given up after ten seconds.
        let (_never, awaited) = source();
        let dropped = awaited.dropped.clone();
        let before = Instant::now();
        let timed_out = cache
            .read(&fixture.provider, "key".into(), awaiting(&reads, awaited), true)
            .await;
        assert_eq!(timed_out, Err("Model catalog request timed out".into()));
        assert_eq!(before.elapsed(), REFRESH_LIMIT);
        assert!(dropped.load(Ordering::SeqCst));

        // Closing cancels a refresh and ends the readers waiting on it.
        let (_never, awaited) = source();
        let started = awaited.started.clone();
        let pending = {
            let cache = cache.clone();
            let provider = fixture.provider.clone();
            let fetch = awaiting(&reads, awaited);
            tokio::spawn(async move { cache.read(&provider, "key".into(), fetch, true).await })
        };
        started.notified().await;
        cache.close().await;
        assert!(pending.await.unwrap().is_err());
        let closed = cache
            .read(
                &fixture.provider,
                "key".into(),
                answering(&reads, Ok(catalog("Late"))),
                false,
            )
            .await;
        assert_eq!(closed, Err("The model catalog cache is closed".into()));
    }
}
