//! When the rate control probes its path, as libwebrtc's probe controller
//! (`probe_controller.cc`) does for a screen share:
//!
//! - With a view's first picture, two probes at 3 and 6 times the estimate.
//! - While the last probe finds the path carries at least 70% of its rate,
//!   another at twice what it found.
//! - While the sender is application-limited (`bwe::alr`), a probe at twice
//!   the estimate 5 seconds after it became so or after the last probe,
//!   whichever is later, sent after a picture, which probes further as the
//!   first ones do.
//! - After a large drop: when the estimate fell below 66% of what it was and
//!   the path recovers within 5 seconds, while the sender is
//!   application-limited or was until 3 seconds ago, one probe at 85% of the
//!   estimate before the drop, at most every 5 seconds. A drop a still
//!   picture's next to nothing caused comes back at once; one the path
//!   caused fails its probe and stays.
//! - No probe above the codec level's bit rate, which no encoder is asked for.
//!
//! A text's pictures rarely spend their budget, and the delay-based estimate
//! rises only to 1.5 times what they spend, so without probes it stays where
//! it started even on a loopback path. A probe is padding sent at its rate
//! for 15 ms, in 5 chunks at the least, which the page acknowledges and drops
//! (`bwe::probe` measures it). A still screen sends no pictures, so it sends
//! no periodic probes either.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use demi_live_view_protocol::{CHUNK_BYTES, VIDEO_MAX_BITRATE};

use crate::bwe::{Bitrate, probe::ProbeCluster};

/// The first probes, as multiples of the estimate.
const START: [f64; 2] = [3.0, 6.0];
/// The share of its rate a probe must find for the next to follow.
const FURTHER_THRESHOLD: f64 = 0.7;
/// The next probe's rate, as a multiple of what the last found.
const FURTHER_SCALE: f64 = 2.0;
/// How long a probe's result is waited for before probing on stops.
const RESULT_WAIT: Duration = Duration::from_secs(1);
/// How long after the sender became application-limited, or after the last
/// probe, a periodic probe follows.
const ALR_INTERVAL: Duration = Duration::from_secs(5);
/// A probe's rate while the sender is application-limited, as a multiple of
/// the estimate.
const ALR_SCALE: f64 = 2.0;
/// An estimate below this share of the one before is a large drop.
const DROP_THRESHOLD: f64 = 0.66;
/// How long after a large drop a recovery probes back.
const DROP_TIMEOUT: Duration = Duration::from_secs(5);
/// How long after the sender stopped being application-limited a recovery
/// still probes back.
const ALR_ENDED_TIMEOUT: Duration = Duration::from_secs(3);
/// The least time between two probes back after a drop.
const DROP_PROBE_INTERVAL: Duration = Duration::from_secs(5);
/// The probe back's rate, as a share of the estimate before the drop.
const AFTER_DROP: f64 = 0.85;
/// The share of a probe's rate it is expected to find at the least.
const PROBE_UNCERTAINTY: f64 = 0.05;
/// How long a probe sends at its rate.
const DURATION: Duration = Duration::from_millis(15);
/// The fewest chunks a probe takes, so that its measure counts
/// (`bwe::probe` needs 4 of them received).
const MIN_CHUNKS: usize = 5;

/// When the rate control probes, and the probes planned.
#[derive(Debug, Default)]
pub(crate) struct Probes {
    next_id: u32,
    started: bool,
    /// The probe whose result decides whether another follows, its rate,
    /// and until when it is waited for.
    awaiting: Option<(u32, Bitrate, Instant)>,
    /// When the last probe was planned.
    last_probe: Option<Instant>,
    /// The estimate as it last was.
    estimate: Option<Bitrate>,
    /// The last large drop: when, and the estimate before it.
    drop: Option<(Instant, Bitrate)>,
    /// When the last probe back after a drop was planned.
    probed_back: Option<Instant>,
    planned: VecDeque<ProbeCluster>,
}

impl Probes {
    /// A picture's chunk left at `now`, with the path's estimate and since
    /// when the sender is application-limited, if it is.
    pub fn picture(&mut self, now: Instant, estimate: Bitrate, limited: Option<Instant>) {
        if !self.started {
            self.started = true;
            for scale in START {
                let (id, target) = self.plan(estimate * scale, now);
                self.awaiting = Some((id, target, now + RESULT_WAIT));
            }
            return;
        }
        self.expire(now);
        let Some(since) = limited else {
            return;
        };
        let due = self.last_probe.map_or(since, |last| last.max(since)) + ALR_INTERVAL;
        if self.probing() || now < due {
            return;
        }
        let (id, target) = self.plan(estimate * ALR_SCALE, now);
        self.awaiting = Some((id, target, now + RESULT_WAIT));
    }

    /// The estimate is `estimate` at `now`.
    pub fn estimate(&mut self, estimate: Bitrate, now: Instant) {
        if let Some(before) = self.estimate
            && estimate.as_f64() < before.as_f64() * DROP_THRESHOLD
        {
            self.drop = Some((now, before));
        }
        self.estimate = Some(estimate);
    }

    /// The path recovered from an overuse at `now`; `limited` says whether
    /// the sender is application-limited, `limited_until` when it last
    /// stopped being so.
    pub fn recovered(&mut self, now: Instant, limited: bool, limited_until: Option<Instant>) {
        self.expire(now);
        let (Some((dropped, before)), Some(estimate)) = (self.drop, self.estimate) else {
            return;
        };
        let lately_limited = limited_until.is_some_and(|until| now.saturating_duration_since(until) < ALR_ENDED_TIMEOUT);
        if !(limited || lately_limited) || self.probing() {
            return;
        }
        let back = before * AFTER_DROP;
        if back.as_f64() * (1.0 - PROBE_UNCERTAINTY) > estimate.as_f64()
            && now.saturating_duration_since(dropped) < DROP_TIMEOUT
            && self.probed_back.is_none_or(|at| now.saturating_duration_since(at) > DROP_PROBE_INTERVAL)
        {
            self.probed_back = Some(now);
            // One probe, which probes no further.
            self.plan(back, now);
        }
    }

    /// Probe `cluster` found the path carries `measured` at `now`.
    pub fn result(&mut self, cluster: &ProbeCluster, measured: Bitrate, now: Instant) {
        let Some((id, target, _)) = self.awaiting else {
            return;
        };
        if id != cluster.id {
            return;
        }
        self.awaiting = None;
        if measured.as_f64() >= target.as_f64() * FURTHER_THRESHOLD
            && target.as_f64() < f64::from(VIDEO_MAX_BITRATE)
        {
            let (id, target) = self.plan(measured * FURTHER_SCALE, now);
            self.awaiting = Some((id, target, now + RESULT_WAIT));
        }
    }

    /// The next probe to send.
    pub fn take(&mut self) -> Option<ProbeCluster> {
        self.planned.pop_front()
    }

    /// Whether a probe is planned or its result awaited.
    fn probing(&self) -> bool {
        self.awaiting.is_some() || !self.planned.is_empty()
    }

    /// Stops waiting for a result that is late at `now`.
    fn expire(&mut self, now: Instant) {
        if self.awaiting.is_some_and(|(_, _, until)| now >= until) {
            self.awaiting = None;
        }
    }

    /// Plans a probe at `rate`, at most the codec level's, at `now`; answers
    /// its ID and rate. Any probe puts the next periodic one
    /// [`ALR_INTERVAL`] off, as libwebrtc's does, so that a periodic probe
    /// at a dropped estimate does not follow a probe back at once.
    fn plan(&mut self, rate: Bitrate, now: Instant) -> (u32, Bitrate) {
        self.last_probe = Some(now);
        let target = rate.min(Bitrate::bps(u64::from(VIDEO_MAX_BITRATE)));
        let bytes = target * DURATION;
        let chunks = (bytes.as_bytes_usize().div_ceil(CHUNK_BYTES)).max(MIN_CHUNKS);
        self.next_id = self.next_id.wrapping_add(1);
        self.planned.push_back(ProbeCluster {
            id: self.next_id,
            target,
            chunks,
            bytes,
        });
        (self.next_id, target)
    }
}
