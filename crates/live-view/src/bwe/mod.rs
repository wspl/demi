//! The delay-based half of Google Congestion Control (GoogCC) and its probe
//! estimator, ported from str0m 0.24.1's `src/bwe/` with their algorithm
//! unchanged but for the backoff of an application-limited sender below,
//! and what str0m leaves out of libwebrtc's handling of an
//! application-limited sender (`alr`): once the sender sends at its rate
//! again, the acknowledged bit rate follows it at once, as libwebrtc's
//! `AcknowledgedBitrateEstimator::SetAlrEndedTime` has it. A still picture
//! sends next to nothing, and a decrease that took that rate as what the path
//! carries would drop the estimate to its floor.
//!
//! An overuse while the sender is application-limited backs off from the
//! estimate, not from the acknowledged rate, as libwebrtc's
//! WebRTC-Bwe-AlrLimitedBackoff did and as BBR keeps a sample taken while
//! the sender was application-limited from lowering its estimate. The page
//! stamps a chunk when its main thread takes it, so a page busy with a layout
//! looks to the trendline like a queue on the path. Only when the path
//! delivered clearly less than it was sent, the acknowledged rate measured
//! against the rate those chunks were sent at as libwebrtc's probe estimator
//! tells a saturated link, does the path limit the rate, and an overuse backs
//! off from what it delivered.
//!
//! The port holds the arrival groups, the trendline estimator, the AIMD rate
//! control and the acknowledged bit rate estimator, which str0m ported from
//! libwebrtc's `delay_based_bwe.cc`, and the probe estimator, from its
//! `probe_bitrate_estimator.cc`. When to probe is `live-view`'s own
//! (`probe`).
//!
//! For each chunk the page acknowledged, it takes when the Host sent it, when
//! the page received it and when the acknowledgement arrived. This is what
//! str0m takes from a video track's TWCC feedback. A probe's chunks are
//! padding the page acknowledges like a picture's.
//!
//! Copyright 2022 Martin Algesten
//!
//! Permission is hereby granted, free of charge, to any person obtaining a
//! copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit
//! persons to whom the Software is furnished to do so, subject to the
//! following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
//! NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//! OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
//! USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::{
    cmp::Ordering,
    fmt,
    sync::LazyLock,
    time::{Duration, Instant},
};

pub use str0m_proto::{Bitrate, DataSize};

mod acked_bitrate_estimator;
pub mod alr;
mod average;
mod delay;
mod macros;
pub mod probe;
mod time;

use acked_bitrate_estimator::AckedBitrateEstimator;
use delay::DelayController;

/// The share of the rate the acknowledged chunks were sent at below which
/// the path delivered clearly less than it was given: it limits the rate.
/// libwebrtc's probe estimator takes a link as saturated below 90% of the
/// sent rate. The page's receive times count its main thread's delay too, so
/// a page that falls behind looks like a slower path: below half, it takes
/// fewer than half the chunks, as one busy for seconds does, and fewer
/// pictures then are what it can show.
const SATURATED: f64 = 0.5;

/// str0m's windows for the acknowledged bit rate (`bwe/mod.rs`).
const INITIAL_BITRATE_WINDOW: Duration = Duration::from_millis(500);
const BITRATE_WINDOW: Duration = Duration::from_millis(150);

/// What orders acknowledged chunks that the page received at the same time:
/// the order the Host sent them in, as TWCC's sequence numbers do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TwccSeq(u64);

impl From<u64> for TwccSeq {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// A chunk the page acknowledged.
#[derive(Debug, Copy, Clone)]
pub struct AckedPacket {
    /// The order the Host sent it in.
    pub seq_no: TwccSeq,
    /// Its bytes on the wire.
    pub size: DataSize,
    /// When the Host sent it.
    pub local_send_time: Instant,
    /// When the page received it, on a clock of the page's that only its
    /// differences mean anything on.
    pub remote_recv_time: Instant,
    /// When its acknowledgement reached the Host.
    pub local_recv_time: Instant,
}

impl AckedPacket {
    fn rtt(&self) -> Duration {
        self.local_recv_time - self.local_send_time
    }

    fn order_by_receive_time(lhs: &Self, rhs: &Self) -> Ordering {
        if lhs.remote_recv_time != rhs.remote_recv_time {
            lhs.remote_recv_time.cmp(&rhs.remote_recv_time)
        } else if lhs.local_send_time != rhs.local_send_time {
            lhs.local_send_time.cmp(&rhs.local_send_time)
        } else {
            lhs.seq_no.cmp(&rhs.seq_no)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BandwidthUsage {
    Overuse,
    Normal,
    Underuse,
}

impl fmt::Display for BandwidthUsage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BandwidthUsage::Overuse => write!(f, "overuse"),
            BandwidthUsage::Normal => write!(f, "normal"),
            BandwidthUsage::Underuse => write!(f, "underuse"),
        }
    }
}

/// An instant before any the estimator sees, which str0m takes as "never".
fn already_happened() -> Instant {
    static BEGINNING: LazyLock<Instant> = LazyLock::new(|| {
        let now = Instant::now();
        // Up to an hour back, as far as the clock reaches.
        (1..=3600)
            .rev()
            .find_map(|secs| now.checked_sub(Duration::from_secs(secs)))
            .unwrap_or(now)
    });
    *BEGINNING
}

/// The delay-based estimate of what the path carries, as str0m's
/// `SendSideBandwidthEstimator` keeps it without its loss controller.
pub struct DelayBasedEstimate {
    delay_controller: DelayController,
    acked_bitrate_estimator: AckedBitrateEstimator,
    /// The rate the acknowledged chunks were sent at, measured as the
    /// acknowledged rate is, from their send times.
    sent_bitrate_estimator: AckedBitrateEstimator,
    /// When the sender stopped being application-limited, until a chunk
    /// sent after it is acknowledged.
    alr_ended: Option<Instant>,
    /// Whether the sender sends less than the path carries, as of the last
    /// acknowledgements: then an overuse backs off from the estimate.
    limited: bool,
}

impl DelayBasedEstimate {
    pub fn new(initial_bitrate: Bitrate) -> Self {
        Self {
            delay_controller: DelayController::new(initial_bitrate),
            acked_bitrate_estimator: AckedBitrateEstimator::new(
                INITIAL_BITRATE_WINDOW,
                BITRATE_WINDOW,
            ),
            sent_bitrate_estimator: AckedBitrateEstimator::new(
                INITIAL_BITRATE_WINDOW,
                BITRATE_WINDOW,
            ),
            alr_ended: None,
            limited: false,
        }
    }

    /// The sender stopped being application-limited at `now`.
    pub fn alr_ended(&mut self, now: Instant) {
        self.alr_ended = Some(now);
    }

    /// Takes in a batch of acknowledged chunks, as str0m takes a TWCC
    /// report's, with what a probe among them measured and whether the
    /// sender is application-limited, and answers the estimate. A probe's
    /// result becomes the estimate. Answers whether the path recovered from
    /// an overuse among them.
    pub fn update(&mut self, mut acked: Vec<AckedPacket>, probe: Option<Bitrate>, limited: bool, now: Instant) -> bool {
        acked.sort_by(AckedPacket::order_by_receive_time);
        for packet in &acked {
            if self.alr_ended.is_some_and(|ended| packet.local_send_time > ended) {
                self.acked_bitrate_estimator.expect_fast_rate_change();
                self.alr_ended = None;
            }
            self.acked_bitrate_estimator
                .update(packet.remote_recv_time, packet.size);
        }
        let mut by_send = acked.clone();
        by_send.sort_by_key(|packet| packet.local_send_time);
        for packet in &by_send {
            self.sent_bitrate_estimator.update(packet.local_send_time, packet.size);
        }
        let acked_bitrate = self.acked_bitrate_estimator.current_estimate();
        self.limited = limited && !self.path_limited(acked_bitrate);
        self.delay_controller.update(&acked, acked_bitrate, probe, self.limited, now)
    }

    /// When [`Self::handle_timeout`] is due.
    pub fn poll_timeout(&self) -> Instant {
        self.delay_controller.poll_timeout()
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        self.delay_controller
            .handle_timeout(self.acked_bitrate_estimator.current_estimate(), self.limited, now);
    }

    /// Whether an overuse backs off from the estimate, the sender sending
    /// less than the path carries, as of the last acknowledgements.
    pub fn limited(&self) -> bool {
        self.limited
    }

    /// The acknowledged rate as a share of the rate those chunks were sent
    /// at; none before both are measured.
    pub fn delivered_share(&self) -> Option<f64> {
        let acked = self.acked_bitrate_estimator.current_estimate()?;
        let sent = self.sent_bitrate_estimator.current_estimate()?;
        Some(acked.as_f64() / sent.as_f64().max(1.0))
    }

    /// Whether the path delivered clearly less than the sender sent: then it
    /// limits the rate, whatever the sender's own budget left unused, and
    /// what was acknowledged is what it carries.
    fn path_limited(&self, acked: Option<Bitrate>) -> bool {
        match (acked, self.sent_bitrate_estimator.current_estimate()) {
            (Some(acked), Some(sent)) => acked.as_f64() < sent.as_f64() * SATURATED,
            _ => false,
        }
    }

    /// The estimate, which starts at the initial bit rate.
    pub fn estimate(&self) -> Bitrate {
        self.delay_controller
            .last_estimate()
            .expect("the delay controller starts with an estimate")
    }
}
