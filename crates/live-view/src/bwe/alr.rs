//! Whether the sender is application-limited: it sends less than the path
//! carries, as a still picture does. Ported from libwebrtc's
//! `alr_detector.cc` and the `IntervalBudget` it keeps, with their default
//! configuration.

use std::time::{Duration, Instant};

use str0m_proto::{Bitrate, DataSize};

/// The share of the estimate the budget fills at.
const BANDWIDTH_USAGE: f64 = 0.65;
/// The budget's unused share above which the sender is application-limited.
const START_LEVEL: f64 = 0.80;
/// The budget's unused share below which it no longer is.
const STOP_LEVEL: f64 = 0.50;
/// How much sending the budget holds, at its rate.
const WINDOW: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
pub struct AlrDetector {
    /// The budget's rate, in bytes a second.
    rate: f64,
    /// The bytes the budget holds unused; below zero, those sent beyond it.
    remaining: f64,
    last_sent: Option<Instant>,
    started: Option<Instant>,
}

impl AlrDetector {
    /// The path's estimate is `estimate` now.
    pub fn set_estimate(&mut self, estimate: Bitrate) {
        self.rate = estimate.as_f64() * BANDWIDTH_USAGE / 8.0;
        let most = self.most();
        self.remaining = self.remaining.clamp(-most, most);
    }

    /// `bytes` went to the page at `now`.
    pub fn sent(&mut self, bytes: DataSize, now: Instant) {
        // The time the first bytes took is unknown.
        let Some(last) = self.last_sent.replace(now) else {
            return;
        };
        let most = self.most();
        self.remaining = (self.remaining - bytes.as_bytes_f64()).max(-most);
        let earned = self.rate * now.saturating_duration_since(last).as_secs_f64();
        self.remaining = (self.remaining + earned).min(most);
        let level = if most > 0.0 { self.remaining / most } else { 0.0 };
        if level > START_LEVEL && self.started.is_none() {
            self.started = Some(now);
        } else if level < STOP_LEVEL && self.started.is_some() {
            self.started = None;
        }
    }

    /// Since when the sender is application-limited; none while it is not.
    pub fn started(&self) -> Option<Instant> {
        self.started
    }

    fn most(&self) -> f64 {
        self.rate * WINDOW.as_secs_f64()
    }
}
