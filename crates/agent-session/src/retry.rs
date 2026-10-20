//! Transient retries: the agent, not the provider, retries a request that
//! failed in a way waiting can fix. A provider makes one attempt per run and
//! reports a classified failure, so counts, backoff, cancellation, transcript
//! safety and the reports of a retry are the same for HTTP and streaming
//! failures; the one exception is a credential's refresh after an HTTP 401.
//!
//! An attempt is retried only when everything it wrote can be unwound.
//! Reasoning is unwound; text already streamed out is not, because a product
//! may have posted it and a second attempt would post a second answer beside
//! it. Tool calls completed by earlier requests stay and are not run again.

use std::time::Duration;

use demi_provider_common::{ErrorCode, ProviderFailure};

/// When and how long a failed request is retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts per request, the first included.
    pub max_attempts: u32,
    /// The cap of the random wait before the second attempt; it doubles with
    /// each retry.
    pub base_delay: Duration,
    /// The backoff ceiling: no wait is longer, and a vendor wait beyond it
    /// makes the failure terminal at once.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Whether `failure` of attempt `attempt`, counted from 1, is retried
    /// when everything the attempt wrote can be unwound.
    pub(crate) fn retries(&self, attempt: u32, failure: &ProviderFailure) -> bool {
        let transient = matches!(
            failure.code,
            Some(ErrorCode::RateLimit | ErrorCode::Overloaded | ErrorCode::DeviceOffline)
        );
        let outlasts = failure
            .retry_after
            .is_some_and(|wait| wait > self.max_delay);
        attempt < self.max_attempts && transient && !outlasts
    }

    /// The wait after attempt `attempt` failed: the vendor's when it named
    /// one, otherwise a random wait below a cap that doubles with each
    /// retry, never above the ceiling.
    pub(crate) fn delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(wait) = retry_after {
            return wait.min(self.max_delay);
        }
        let doublings = attempt.saturating_sub(1);
        let cap = self
            .base_delay
            .saturating_mul(2_u32.saturating_pow(doublings))
            .min(self.max_delay);
        let cap_ms = u64::try_from(cap.as_millis()).unwrap_or(u64::MAX);
        if cap_ms == 0 {
            return Duration::ZERO;
        }
        Duration::from_millis(rand::random_range(0..cap_ms))
    }
}
