//! The delay-based half of Google Congestion Control (GoogCC), ported from
//! str0m 0.24.1's `src/bwe/` with its algorithm unchanged (`live-view.md`
//! § Delivery): the arrival groups, the trendline estimator, the AIMD rate
//! control and the acknowledged bit rate estimator, which str0m ported from
//! libwebrtc's `delay_based_bwe.cc`. It takes, for each chunk the page
//! acknowledged, when the Host sent it, when the page received it and when
//! the acknowledgement arrived, what str0m takes from a video track's TWCC
//! feedback; there are no padding probes, so the estimate rises by the rate
//! control's own increase.
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
mod average;
mod delay;
mod macros;
mod time;

use acked_bitrate_estimator::AckedBitrateEstimator;
use delay::DelayController;

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
/// `SendSideBandwidthEstimator` keeps it without its loss controller and
/// probes.
pub struct DelayBasedEstimate {
    delay_controller: DelayController,
    acked_bitrate_estimator: AckedBitrateEstimator,
}

impl DelayBasedEstimate {
    pub fn new(initial_bitrate: Bitrate) -> Self {
        Self {
            delay_controller: DelayController::new(initial_bitrate),
            acked_bitrate_estimator: AckedBitrateEstimator::new(
                INITIAL_BITRATE_WINDOW,
                BITRATE_WINDOW,
            ),
        }
    }

    /// Takes in a batch of acknowledged chunks, as str0m takes a TWCC
    /// report's, and answers the estimate.
    pub fn update(&mut self, mut acked: Vec<AckedPacket>, now: Instant) -> Option<Bitrate> {
        acked.sort_by(AckedPacket::order_by_receive_time);
        for packet in &acked {
            self.acked_bitrate_estimator
                .update(packet.remote_recv_time, packet.size);
        }
        let acked_bitrate = self.acked_bitrate_estimator.current_estimate();
        self.delay_controller.update(&acked, acked_bitrate, None, now)
    }

    /// When [`Self::handle_timeout`] is due.
    pub fn poll_timeout(&self) -> Instant {
        self.delay_controller.poll_timeout()
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        self.delay_controller
            .handle_timeout(self.acked_bitrate_estimator.current_estimate(), now);
    }

    /// The estimate, which starts at the initial bit rate.
    pub fn estimate(&self) -> Bitrate {
        self.delay_controller
            .last_estimate()
            .expect("the delay controller starts with an estimate")
    }
}
