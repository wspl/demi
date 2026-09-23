//! The vendor quota of a subscription account (`usage-and-quota.md` § Vendor
//! quota): a family's quota source, and the account's snapshot store that
//! probes and observations update. The snapshot's shape is `core`'s, because
//! the browser receives it.

use std::sync::Arc;

use demi_core::{QuotaPlan, QuotaSnapshot, QuotaWindow};
use futures_util::future::BoxFuture;
use http::{HeaderMap, StatusCode};

/// The quota of the one account a provider stands for.
pub struct ProviderQuota {
    source: Box<dyn QuotaSource>,
    store: Arc<dyn QuotaSnapshotStore>,
}

impl ProviderQuota {
    /// Connects a family's quota source to the account's snapshot store.
    pub fn new(source: Box<dyn QuotaSource>, store: Arc<dyn QuotaSnapshotStore>) -> Self {
        Self { source, store }
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
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeReading {
    pub plan: Option<QuotaPlan>,
    pub account_label: Option<String>,
    pub windows: Vec<QuotaWindow>,
}

/// Why a probe failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuotaError {
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
    /// probe and an observation that arrive together both land.
    fn update(
        &self,
        next: &mut dyn FnMut(Option<&QuotaSnapshot>) -> QuotaSnapshot,
    ) -> Arc<QuotaSnapshot>;
}
