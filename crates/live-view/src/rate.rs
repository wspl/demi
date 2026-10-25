//! What a viewer's path takes, computed from the page's feedback:
//!
//! - The target bit rate comes from the delay-based estimate of Google
//!   Congestion Control, fed with each chunk's send time and the page's receive
//!   time.
//! - The window of bytes that may be unacknowledged is the target times the
//!   least round trip of the last ten seconds plus 60 ms.
//! - The frame rate and resolution are those the target affords a picture of
//!   the panel's size.
//!
//! # Rules
//!
//! - Neither path says when a chunk was sent again, and such a chunk comes back
//!   seconds late. So the least of ten seconds drops the sample once a clean
//!   one arrives. For the first ten seconds the round trip counts at most 300
//!   ms.
//! - The rate control probes the path with padding (`probe`), since the
//!   pictures alone rarely fill it. A rising delay lowers the rate. A loss
//!   whose retransmission only delays a chunk does not.
//! - Below what a picture needs, the sender lowers the frame rate first, then
//!   the resolution.
//! - Viewers of a source share its encoding, so the slowest decides. A blocked
//!   page or stall is a pause, not congestion.
//! - The only ceiling is the codec level's 240 Mbps, past which the encoder
//!   refuses to start.
//! - Watching costs Host CPU, which this adaptation does not measure.

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use demi_live_view_protocol::{ChunkAck, VIDEO_MAX_BITRATE};

use crate::{
    bwe::{
        AckedPacket, Bitrate, DataSize, DelayBasedEstimate,
        probe::{ProbeCluster, ProbeEstimator},
    },
    probe::Probes,
};

/// Where the estimate starts, before the path said anything.
pub(crate) const INITIAL_BITRATE: u32 = 5_000_000;
/// The least the encoder is asked for.
const MIN_BITRATE: u32 = 100_000;
/// How long the least round trip is remembered.
const ROUND_TRIP_SPAN: Duration = Duration::from_secs(10);
/// What the window allows beyond the round trip: a frame's pacing and the
/// page's batching of its acknowledgements.
const WINDOW_SLACK: Duration = Duration::from_millis(60);
/// The most round trip the window counts in a view's first
/// [`ROUND_TRIP_SPAN`]: neither path tells the sender of a chunk it sent
/// again, such as the first key frame's on a lossy link, whose late
/// acknowledgement would make the window seconds long. Past that span the
/// least sample of the span counts whole, so a long path keeps its window.
const WINDOW_ROUND_TRIP: Duration = Duration::from_millis(300);
/// Bits per pixel per frame below which the picture is mush in motion:
/// fewer frames, then fewer pixels.
const LOW_BITS: f64 = 0.05;
/// A lowered frame rate or resolution returns once the target covers it at
/// this many bits: a fifth above `LOW_BITS`, so the two do not alternate
/// with the estimate's own swing, and near enough that a target back where
/// it was before a drop brings the picture back as it was.
const RECOVERED_BITS: f64 = 0.06;
pub(crate) const FRAME_RATES: [u32; 3] = [60, 30, 15];
pub(crate) const SCALES: [f64; 3] = [1.0, 0.75, 0.5];
/// How far from the encoder's bit rate the target moves before the encoder
/// is told: every change would reconfigure it at the estimate's pace.
const RETARGET: f64 = 0.1;

/// A chunk as the viewer's page names it in its acknowledgement: generation,
/// sequence (a picture's, or a probe's for padding), index, and whether it
/// is padding.
pub(crate) type ChunkId = (u32, u32, u16, bool);

struct Sent {
    at: Instant,
    bytes: usize,
    /// The order chunks went out in, which breaks ties of receive times.
    order: u64,
    /// The probe it is padding of.
    probe: Option<u32>,
}

pub(crate) struct Rate {
    estimate: DelayBasedEstimate,
    flight: HashMap<ChunkId, Sent>,
    in_flight: usize,
    sent: u64,
    round_trips: VecDeque<(Instant, Duration)>,
    /// When the view's first chunk went; none before.
    started: Option<Instant>,
    /// Whether the round trips of the view's first span are all there are.
    early: bool,
    /// The page's clock against the Host's: a local instant and the page's
    /// milliseconds at the first acknowledgement.
    clock: Option<(Instant, f64)>,
    pixels: f64,
    rate: usize,
    scale: usize,
    probes: Probes,
    probe_estimator: ProbeEstimator,
}

impl Rate {
    /// A picture of `pixels` device pixels at full resolution.
    pub fn new(pixels: u64) -> Self {
        let mut rate = Self {
            estimate: DelayBasedEstimate::new(Bitrate::bps(u64::from(INITIAL_BITRATE))),
            flight: HashMap::new(),
            in_flight: 0,
            sent: 0,
            round_trips: VecDeque::new(),
            started: None,
            early: true,
            clock: None,
            pixels: 0.0,
            rate: 0,
            scale: 0,
            probes: Probes::default(),
            probe_estimator: ProbeEstimator::default(),
        };
        rate.resize(pixels);
        rate
    }

    /// The panel's picture has `pixels` device pixels now.
    pub fn resize(&mut self, pixels: u64) {
        self.pixels = pixels as f64;
        self.fit();
    }

    /// The bit rate the path takes, as far as the codec's level carries.
    pub fn target(&self) -> u32 {
        (self.estimate.estimate().as_f64().round() as u64)
            .clamp(u64::from(MIN_BITRATE), u64::from(VIDEO_MAX_BITRATE)) as u32
    }

    pub fn fps(&self) -> u32 {
        FRAME_RATES[self.rate]
    }

    /// The fraction of the picture's width and height to capture.
    pub fn scale(&self) -> f64 {
        SCALES[self.scale]
    }

    /// Whether the encoder, told `applied`, should hear the target now.
    pub fn retarget(&self, applied: u32) -> bool {
        let target = f64::from(self.target());
        (target - f64::from(applied)).abs() > f64::from(applied) * RETARGET
    }

    /// The least round trip of the last ten seconds; none before the first.
    pub fn round_trip(&self) -> Option<Duration> {
        self.round_trips.iter().map(|(_, round_trip)| *round_trip).min()
    }

    /// The bytes that may be unacknowledged: the target over the least
    /// round trip, at most [`WINDOW_ROUND_TRIP`] early in the view, and the
    /// slack.
    pub fn window(&self) -> usize {
        let round_trip = self.round_trip().unwrap_or_default();
        let round_trip = if self.early { round_trip.min(WINDOW_ROUND_TRIP) } else { round_trip };
        let span = round_trip + WINDOW_SLACK;
        (f64::from(self.target()) / 8.0 * span.as_secs_f64()) as usize
    }

    /// Bytes sent and not acknowledged.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// Whether a frame of about `next` bytes fits the window beside
    /// `queued` bytes waiting to be sent; always when nothing is under way,
    /// so the window is at least one frame.
    pub fn room(&self, queued: usize, next: usize) -> bool {
        let under_way = self.in_flight + queued;
        under_way == 0 || under_way + next <= self.window()
    }

    /// Chunk `id` of `bytes` bytes went to the page at `now`.
    pub fn sent(&mut self, id: ChunkId, bytes: usize, now: Instant) {
        self.started.get_or_insert(now);
        self.sent += 1;
        let order = self.sent;
        let (_, sequence, _, padding) = id;
        let probe = padding.then_some(sequence);
        if !padding {
            self.probes.picture(bytes, now, self.estimate.estimate());
        }
        if let Some(replaced) = self.flight.insert(id, Sent { at: now, bytes, order, probe }) {
            self.in_flight -= replaced.bytes;
        }
        self.in_flight += bytes;
    }

    /// The next probe to send, which is measured from then on.
    pub fn next_probe(&mut self) -> Option<ProbeCluster> {
        let cluster = self.probes.take()?;
        self.probe_estimator.start(cluster);
        Some(cluster)
    }

    /// Probe `id`'s last chunk went at `now`.
    pub fn probe_sent(&mut self, id: u32, now: Instant) {
        self.probe_estimator.end(id, now);
    }

    /// The page acknowledged `acks` in one message, which arrived at `now`.
    pub fn acknowledged(&mut self, acks: &[ChunkAck], now: Instant) {
        let mut acked = Vec::with_capacity(acks.len());
        // What the latest probe among them measured.
        let mut probed = None;
        for ack in acks {
            // A chunk acknowledged twice, or of a stream before this one, is
            // nothing the path still holds.
            let Some(sent) = self.flight.remove(&(ack.generation, ack.sequence, ack.chunk, ack.padding)) else {
                continue;
            };
            self.in_flight -= sent.bytes;
            let round_trip = now.saturating_duration_since(sent.at);
            self.early = self
                .started
                .is_none_or(|started| now.saturating_duration_since(started) < ROUND_TRIP_SPAN);
            self.round_trips.push_back((now, round_trip));
            let packet = AckedPacket {
                seq_no: sent.order.into(),
                size: DataSize::bytes(sent.bytes as i64),
                local_send_time: sent.at,
                remote_recv_time: self.page_time(ack.received, now),
                local_recv_time: now,
            };
            if let Some(id) = sent.probe
                && let Some(found) = self.probe_estimator.update(id, &packet)
            {
                probed = Some(found);
            }
            acked.push(packet);
        }
        while self
            .round_trips
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > ROUND_TRIP_SPAN)
        {
            self.round_trips.pop_front();
        }
        if !acked.is_empty() {
            self.estimate.update(acked, probed.map(|(_, measured)| measured), now);
        }
        if let Some((cluster, measured)) = probed {
            tracing::debug!(
                event = "live_view.probed",
                target = cluster.target.as_u64(),
                measured = measured.as_u64(),
                "a live view's probe measured its path"
            );
            self.probes.result(&cluster, measured, now);
        }
        self.probe_estimator.handle_timeout(now);
        if now >= self.estimate.poll_timeout() {
            self.estimate.handle_timeout(now);
        }
        self.fit();
    }

    /// The page's `received` milliseconds as an instant on the Host's clock,
    /// which the estimate takes only the differences of.
    fn page_time(&mut self, received: f64, now: Instant) -> Instant {
        // A minute of room before the first acknowledgement's time: a later
        // message can acknowledge a chunk received before it.
        const EARLIER: Duration = Duration::from_secs(60);
        let (origin, page) = *self.clock.get_or_insert((now, received));
        let base = origin.checked_sub(EARLIER).unwrap_or(origin);
        let offset = ((received - page) / 1000.0 + EARLIER.as_secs_f64()).max(0.0);
        base + Duration::from_secs_f64(offset)
    }

    fn bits(&self, fps: u32, scale: f64) -> f64 {
        f64::from(self.target()) / (self.pixels * scale * scale * f64::from(fps)).max(1.0)
    }

    /// Fewer frames, then fewer pixels, while the target affords too few
    /// bits; pixels come back before frames, the reverse of their loss.
    fn fit(&mut self) {
        loop {
            if self.bits(self.fps(), self.scale()) < LOW_BITS {
                if self.rate + 1 < FRAME_RATES.len() {
                    self.rate += 1;
                    continue;
                }
                if self.scale + 1 < SCALES.len() {
                    self.scale += 1;
                    continue;
                }
            } else if self.scale > 0
                && self.bits(self.fps(), SCALES[self.scale - 1]) >= RECOVERED_BITS
            {
                self.scale -= 1;
                continue;
            } else if self.scale == 0
                && self.rate > 0
                && self.bits(FRAME_RATES[self.rate - 1], 1.0) >= RECOVERED_BITS
            {
                self.rate -= 1;
                continue;
            }
            return;
        }
    }
}
