//! The vendor quota of a subscription account (`usage-and-quota.md` § Vendor
//! quota): a family's quota source, which probes the vendor's usage endpoint
//! and reads the quota a live response reports; the account's snapshot
//! store; the one merge both paths update the snapshot by; and the helpers a
//! source reads a vendor's windows with. The snapshot's shape is `core`'s,
//! because the browser receives it.

use std::sync::{Arc, Mutex, PoisonError};

use demi_core::{
    Clock, QuotaPlan, QuotaSeverity, QuotaSnapshot, QuotaWindow, SnapshotSource, Timestamp,
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, StatusCode};

/// The quota of the one account a provider stands for. Its probes and
/// observations read and write that account's snapshot only, so a request
/// that outlives a change of the active account still credits the account
/// that made it.
pub struct ProviderQuota {
    source: Box<dyn QuotaSource>,
    store: Arc<dyn QuotaSnapshotStore>,
    clock: Arc<dyn Clock>,
}

impl ProviderQuota {
    /// Connects a family's quota source to the account's snapshot store.
    pub fn new(
        source: Box<dyn QuotaSource>,
        store: Arc<dyn QuotaSnapshotStore>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            source,
            store,
            clock,
        }
    }

    /// The account's kept snapshot. Reading it never probes.
    pub fn latest(&self) -> Option<Arc<QuotaSnapshot>> {
        self.store.latest()
    }

    /// What a probe of the account costs; `None` when the family cannot
    /// probe.
    pub fn probe_cost(&self) -> Option<ProbeCost> {
        self.source.probe_cost()
    }

    /// Asks the vendor's usage endpoint about the account and merges the
    /// answer into its snapshot: the probe sets the plan and the account
    /// label and replaces the windows it names. A family whose probe would
    /// spend an inference request is never probed.
    pub async fn probe(&self) -> Result<Arc<QuotaSnapshot>, QuotaError> {
        match self.source.probe_cost() {
            None => return Err(QuotaError::Unsupported),
            Some(ProbeCost::Inference) => return Err(QuotaError::RequiresInference),
            Some(ProbeCost::Free) => {}
        }
        let reading = self.source.probe().await?;
        let observed_at = self.clock.now();
        let mut reading = Some(reading);
        Ok(self.store.update(&mut |previous| {
            // The store calls the merge once; a second call would find the
            // reading taken and merge an empty one.
            let reading = reading.take().unwrap_or_default();
            merge(previous, Update::Probe(reading), observed_at)
        }))
    }

    /// Merges the windows a live response reports into the account's
    /// snapshot, keeping its plan and label. A response the source cannot
    /// read changes nothing, and observing never fails the request.
    pub fn observe(&self, observation: Observation<'_>) {
        let Some(windows) = self.source.observe(observation) else {
            return;
        };
        let observed_at = self.clock.now();
        let mut windows = Some(windows);
        self.store.update(&mut |previous| {
            let windows = windows.take().unwrap_or_default();
            merge(previous, Update::Observation(windows), observed_at)
        });
    }
}

/// How a family reads its vendor's quota.
pub trait QuotaSource: Send + Sync {
    /// What a probe costs; `None` when the family cannot probe.
    fn probe_cost(&self) -> Option<ProbeCost>;

    /// Asks the vendor's usage endpoint about the account.
    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>>;

    /// The windows a live response reports. It is pure and never fails a
    /// run: a response it cannot read gives `None`.
    fn observe(&self, observation: Observation<'_>) -> Option<Vec<QuotaWindow>>;
}

/// What a probe costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeCost {
    /// It reads a usage endpoint and spends no inference.
    Free,
    /// It spends an inference request, which Demi never runs as a probe.
    Inference,
}

/// A live response that may carry quota.
#[derive(Debug, Clone, Copy)]
pub enum Observation<'a> {
    /// An HTTP response of the vendor's service, a refusal included.
    Response {
        status: StatusCode,
        headers: &'a HeaderMap,
    },
    /// A line of a CLI's output.
    CliLine(&'a serde_json::Value),
}

/// What a probe read about the account.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProbeReading {
    pub plan: Option<QuotaPlan>,
    pub account_label: Option<String>,
    pub windows: Vec<QuotaWindow>,
}

/// Why a probe failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuotaError {
    /// The family has no usage endpoint to probe.
    #[error("this provider cannot read its usage")]
    Unsupported,
    /// The family cannot probe without spending an inference request.
    #[error("reading this provider's usage requires an inference request")]
    RequiresInference,
    #[error("{0}")]
    Unauthenticated(String),
    /// The usage endpoint did not answer, or refused.
    #[error("{0}")]
    Unavailable(String),
    /// The usage endpoint answered with a payload that cannot be read.
    #[error("{0}")]
    Invalid(String),
}

/// Where an account's snapshot is kept.
pub trait QuotaSnapshotStore: Send + Sync {
    fn latest(&self) -> Option<Arc<QuotaSnapshot>>;

    /// Reads, merges and writes the account's snapshot as one step, so that a
    /// probe and an observation that arrive together both land. `next` is
    /// called once.
    fn update(
        &self,
        next: &mut dyn FnMut(Option<&QuotaSnapshot>) -> QuotaSnapshot,
    ) -> Arc<QuotaSnapshot>;
}

/// A snapshot store held in memory, such as for a provider built during a
/// login, whose account has no record yet, and for tests.
#[derive(Debug, Default)]
pub struct MemorySnapshots {
    /// A `std` mutex: probes and observations arrive from any thread, and
    /// the merge under it never awaits.
    latest: Mutex<Option<Arc<QuotaSnapshot>>>,
}

impl MemorySnapshots {
    pub fn new() -> Self {
        Self::default()
    }
}

impl QuotaSnapshotStore for MemorySnapshots {
    fn latest(&self) -> Option<Arc<QuotaSnapshot>> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn update(
        &self,
        next: &mut dyn FnMut(Option<&QuotaSnapshot>) -> QuotaSnapshot,
    ) -> Arc<QuotaSnapshot> {
        // A merge never panics halfway, so a poisoned lock holds a whole
        // snapshot.
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        let snapshot = Arc::new(next(latest.as_deref()));
        *latest = Some(snapshot.clone());
        snapshot
    }
}

/// What updates a snapshot.
enum Update {
    Probe(ProbeReading),
    Observation(Vec<QuotaWindow>),
}

/// The one merge of `usage-and-quota.md` § Probe and observation: a probe
/// sets the plan and the account label; an observation keeps them; either
/// replaces the windows it names, keeps the others, and adds new ones after
/// them in the order it read them.
fn merge(previous: Option<&QuotaSnapshot>, update: Update, observed_at: Timestamp) -> QuotaSnapshot {
    let (source, plan, account_label, windows) = match update {
        Update::Probe(reading) => (
            SnapshotSource::Probe,
            reading.plan,
            reading.account_label,
            reading.windows,
        ),
        Update::Observation(windows) => (
            SnapshotSource::Observation,
            previous.and_then(|snapshot| snapshot.plan.clone()),
            previous.and_then(|snapshot| snapshot.account_label.clone()),
            windows,
        ),
    };
    let mut merged: Vec<QuotaWindow> = previous.map(|snapshot| snapshot.windows.clone()).unwrap_or_default();
    for window in windows {
        match merged.iter_mut().find(|kept| kept.id == window.id) {
            Some(kept) => *kept = window,
            None => merged.push(window),
        }
    }
    QuotaSnapshot {
        observed_at,
        source,
        plan,
        account_label,
        windows: merged,
    }
}

/// A reported share used as a window's percentage: a share above 100 counts
/// as 100 and one below 0 as 0; `None` for a value that is not a finite
/// number.
pub fn clamp_used_percent(value: f64) -> Option<f64> {
    value.is_finite().then(|| value.clamp(0.0, 100.0))
}

/// The share of `limit` that `used` is, as a percentage; `None` unless the
/// limit is above zero.
pub fn used_percent_from_ratio(used: f64, limit: f64) -> Option<f64> {
    if !used.is_finite() || !limit.is_finite() || limit <= 0.0 {
        return None;
    }
    clamp_used_percent(used / limit * 100.0)
}

/// A window's severity by its share used: `critical` from 95, `warning`
/// from 80, `normal` below that, and none when the share is unknown.
pub fn severity(used_percent: Option<f64>) -> Option<QuotaSeverity> {
    let used = used_percent?;
    Some(if used >= 95.0 {
        QuotaSeverity::Critical
    } else if used >= 80.0 {
        QuotaSeverity::Warning
    } else {
        QuotaSeverity::Normal
    })
}

/// A reset time a vendor field states in Unix seconds, a fraction counted to
/// the millisecond; `None` for a value outside the range of times.
pub fn unix_seconds(seconds: f64) -> Option<Timestamp> {
    let milliseconds = (seconds * 1000.0).floor();
    // A finite value within i64 converts exactly enough for a time; the
    // range check of `from_millisecond` refuses the rest.
    if !milliseconds.is_finite() || milliseconds.abs() >= 9.0e15 {
        return None;
    }
    Timestamp::from_millisecond(milliseconds as i64).ok()
}

/// A reset time a vendor field states as an RFC 3339 time, such as
/// `2026-08-21T10:45:24.951512+00:00`, to the millisecond; `None` for any
/// other text.
pub fn rfc3339(text: &str) -> Option<Timestamp> {
    let time: jiff::Timestamp = text.trim().parse().ok()?;
    Some(Timestamp::truncate(time))
}

/// A header read as a number: its trimmed text as a finite decimal number;
/// `None` when the header is absent, blank or not a number. Headers carry no
/// type, so a vendor that omits one or sends a word means no value.
pub fn header_number(headers: &HeaderMap, name: &str) -> Option<f64> {
    let text = headers.get(name)?.to_str().ok()?.trim();
    let number: f64 = text.parse().ok()?;
    number.is_finite().then_some(number)
}
