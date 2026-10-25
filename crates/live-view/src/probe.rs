//! When the rate control probes its path, as libwebrtc's probe controller
//! (`probe_controller.cc`) does for a screen share:
//!
//! - With a view's first picture, two probes at 3 and 6 times the estimate.
//! - While the last probe finds the path carries at least 70% of its rate,
//!   another at twice what it found.
//! - While the pictures spend less than 65% of the estimate, a probe at twice
//!   the estimate every 5 seconds, after a picture, which probes further as
//!   the first ones do.
//! - No probe above the codec level's bit rate, which no encoder is asked for.
//!
//! A text's pictures rarely spend their budget, and the delay-based estimate
//! rises only to 1.5 times what they spend, so without probes it stays where
//! it started even on a loopback path. A probe is padding sent at its rate
//! for 15 ms, in 5 chunks at the least, which the page acknowledges and drops
//! (`bwe::probe` measures it). A still screen sends no pictures, so it sends
//! no probes either.

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
/// The least time between two probes while the pictures leave room.
const ALR_INTERVAL: Duration = Duration::from_secs(5);
/// A probe's rate while the pictures leave room, as a multiple of the
/// estimate.
const ALR_SCALE: f64 = 2.0;
/// The span the pictures' rate is measured over.
const ALR_SPAN: Duration = Duration::from_millis(500);
/// The share of the estimate below which the pictures leave room.
const ALR_RATIO: f64 = 0.65;
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
    /// The next probe while the pictures leave room, not before.
    next_alr: Option<Instant>,
    /// The pictures' bytes sent over the last [`ALR_SPAN`].
    sent: VecDeque<(Instant, usize)>,
    planned: VecDeque<ProbeCluster>,
}

impl Probes {
    /// A picture's chunk of `bytes` left at `now`, with the path's estimate.
    pub fn picture(&mut self, bytes: usize, now: Instant, estimate: Bitrate) {
        self.sent.push_back((now, bytes));
        while self
            .sent
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > ALR_SPAN)
        {
            self.sent.pop_front();
        }
        if !self.started {
            self.started = true;
            self.next_alr = Some(now + ALR_INTERVAL);
            for scale in START {
                let (id, target) = self.plan(estimate * scale);
                self.awaiting = Some((id, target, now + RESULT_WAIT));
            }
            return;
        }
        if self.awaiting.is_some_and(|(_, _, until)| now >= until) {
            self.awaiting = None;
        }
        if self.awaiting.is_some() || !self.planned.is_empty() || self.next_alr.is_some_and(|at| now < at) {
            return;
        }
        self.next_alr = Some(now + ALR_INTERVAL);
        let spent: usize = self.sent.iter().map(|(_, bytes)| bytes).sum();
        let rate = spent as f64 * 8.0 / ALR_SPAN.as_secs_f64();
        if rate < estimate.as_f64() * ALR_RATIO {
            let (id, target) = self.plan(estimate * ALR_SCALE);
            self.awaiting = Some((id, target, now + RESULT_WAIT));
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
            let (id, target) = self.plan(measured * FURTHER_SCALE);
            self.awaiting = Some((id, target, now + RESULT_WAIT));
        }
    }

    /// The next probe to send.
    pub fn take(&mut self) -> Option<ProbeCluster> {
        self.planned.pop_front()
    }

    /// Plans a probe at `rate`, at most the codec level's; answers its ID
    /// and rate.
    fn plan(&mut self, rate: Bitrate) -> (u32, Bitrate) {
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
