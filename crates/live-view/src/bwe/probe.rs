//! The bit rate a probe measured, ported from str0m 0.24.1's
//! `src/bwe/probe/estimator.rs`, which ports libwebrtc's
//! `probe_bitrate_estimator.cc`, with one change: a probe measures once.
//!
//! A probe is a burst of chunks at a rate. What the page received of it says
//! what the path carried: the least of the rate the chunks left at and the
//! rate they arrived at, or a fraction of the arrival rate when that is
//! clearly below the sending one, since the path was then full.
//!
//! libwebrtc measures again at every chunk that arrives, since a padding
//! packet that is lost never does. A chunk here is sent again until it
//! arrives, so a lost one would arrive hundreds of milliseconds late and
//! stretch the arrival rate to a fraction of the path's. A probe's result is
//! therefore the first it measures, once enough of its chunks arrived; a
//! chunk after that is not counted.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use super::{AckedPacket, Bitrate, DataSize};

/// The least share of a probe's chunks the page must have received.
const MIN_RECEIVED_PROBES_RATIO: f64 = 0.80;
/// The least share of a probe's bytes the page must have received.
const MIN_RECEIVED_BYTES_RATIO: f64 = 0.80;
/// The fewest chunks a probe's result counts (libwebrtc's `kMinClusterSize`).
const MIN_CLUSTER_SIZE: usize = 4;
/// The longest a probe's chunks may take to leave or to arrive
/// (`kMaxProbeInterval`).
const MAX_PROBE_INTERVAL: Duration = Duration::from_secs(1);
/// The most the arrival rate may exceed the sending rate by
/// (`kMaxValidRatio`).
const MAX_VALID_RATIO: f64 = 2.0;
/// Below this share of the sending rate, the arrival rate says the path was
/// full (`kMinRatioForUnsaturatedLink`).
const MIN_RATIO_FOR_UNSATURATED_LINK: f64 = 0.9;
/// The share of a full path's arrival rate taken as what it carries
/// (`kTargetUtilizationFraction`).
const TARGET_UTILIZATION_FRACTION: f64 = 0.95;
/// How long after its last chunk left a probe still takes acknowledgements.
const CLUSTER_HISTORY: Duration = Duration::from_secs(1);

/// A probe as it was planned.
#[derive(Debug, Clone, Copy)]
pub struct ProbeCluster {
    pub id: u32,
    /// The rate it is sent at.
    pub target: Bitrate,
    /// The chunks it is sent in.
    pub chunks: usize,
    /// The bytes it is sent in.
    pub bytes: DataSize,
}

/// The probes under way, and what each measured.
#[derive(Debug, Default)]
pub struct ProbeEstimator {
    states: VecDeque<State>,
}

#[derive(Debug)]
struct State {
    cluster: ProbeCluster,
    /// When its acknowledgements stop counting; none while it is sent.
    finalize_at: Option<Instant>,
    first_send: Option<Instant>,
    last_send: Option<Instant>,
    /// The bytes of the chunk sent last, which the sending rate leaves out.
    size_last_send: DataSize,
    first_receive: Option<Instant>,
    last_receive: Option<Instant>,
    /// The bytes of the chunk received first, which the arrival rate leaves
    /// out.
    size_first_receive: DataSize,
    bytes: DataSize,
    count: usize,
    /// Whether it measured the path, after which it counts no chunk.
    measured: bool,
}

impl ProbeEstimator {
    /// `cluster` is being sent.
    pub fn start(&mut self, cluster: ProbeCluster) {
        self.states.push_back(State {
            cluster,
            finalize_at: None,
            first_send: None,
            last_send: None,
            size_last_send: DataSize::ZERO,
            first_receive: None,
            last_receive: None,
            size_first_receive: DataSize::ZERO,
            bytes: DataSize::ZERO,
            count: 0,
            measured: false,
        });
    }

    /// Probe `id`'s last chunk left at `now`.
    pub fn end(&mut self, id: u32, now: Instant) {
        if let Some(state) = self.states.iter_mut().find(|state| state.cluster.id == id) {
            state.finalize_at = Some(now + CLUSTER_HISTORY);
        }
    }

    /// The page acknowledged `packet` of probe `id`: what the probe
    /// measured, with its plan, when this chunk completes its measure.
    pub fn update(&mut self, id: u32, packet: &AckedPacket) -> Option<(ProbeCluster, Bitrate)> {
        let state = self.states.iter_mut().find(|state| state.cluster.id == id && !state.measured)?;
        state.update(packet);
        let bitrate = state.bitrate()?;
        state.measured = true;
        Some((state.cluster, bitrate))
    }

    /// Forgets the probes whose acknowledgements stopped counting.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.states
            .retain(|state| state.finalize_at.is_none_or(|at| now < at));
    }
}

impl State {
    fn update(&mut self, packet: &AckedPacket) {
        let size = packet.size;
        let send = packet.local_send_time;
        let receive = packet.remote_recv_time;
        let first = self.first_send.get_or_insert(send);
        *first = (*first).min(send);
        let last = self.last_send.get_or_insert(send);
        if send >= *last {
            *last = send;
            self.size_last_send = size;
        }
        let first = self.first_receive.get_or_insert(receive);
        if receive <= *first {
            *first = receive;
            self.size_first_receive = size;
        }
        let last = self.last_receive.get_or_insert(receive);
        *last = (*last).max(receive);
        self.bytes += size;
        self.count += 1;
    }

    fn bitrate(&self) -> Option<Bitrate> {
        if self.count < MIN_CLUSTER_SIZE
            || (self.count as f64) < self.cluster.chunks as f64 * MIN_RECEIVED_PROBES_RATIO
            || self.bytes.as_bytes_f64() < self.cluster.bytes.as_bytes_f64() * MIN_RECEIVED_BYTES_RATIO
        {
            return None;
        }
        let send = self.last_send?.saturating_duration_since(self.first_send?);
        let receive = self.last_receive?.saturating_duration_since(self.first_receive?);
        if send.is_zero() || send > MAX_PROBE_INTERVAL || receive.is_zero() || receive > MAX_PROBE_INTERVAL {
            return None;
        }
        let sent = self.bytes.saturating_sub(self.size_last_send);
        let received = self.bytes.saturating_sub(self.size_first_receive);
        if sent <= DataSize::ZERO || received <= DataSize::ZERO {
            return None;
        }
        let send_rate = sent / send;
        let receive_rate = received / receive;
        if receive_rate.as_f64() / send_rate.as_f64() > MAX_VALID_RATIO {
            return None;
        }
        if receive_rate.as_f64() < send_rate.as_f64() * MIN_RATIO_FOR_UNSATURATED_LINK {
            return Some(receive_rate * TARGET_UTILIZATION_FRACTION);
        }
        Some(send_rate.min(receive_rate))
    }
}
