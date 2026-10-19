//! What a viewer's path takes (`live-view.md` § Delivery): the target bit
//! rate from the delay-based estimate of Google Congestion Control, fed with
//! each chunk's send time and the page's receive time; the window of bytes
//! that may be unacknowledged, the target times the least round trip of the
//! last ten seconds plus 60 ms; and the frame rate and resolution the target
//! affords a picture of the panel's size.

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use demi_live_view_protocol::{ChunkAck, VIDEO_MAX_BITRATE};

use crate::bwe::{AckedPacket, Bitrate, DataSize, DelayBasedEstimate};

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
/// sequence and index.
pub(crate) type ChunkId = (u32, u32, u16);

struct Sent {
    at: Instant,
    bytes: usize,
    /// The order chunks went out in, which breaks ties of receive times.
    order: u64,
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
        if let Some(replaced) = self.flight.insert(id, Sent { at: now, bytes, order }) {
            self.in_flight -= replaced.bytes;
        }
        self.in_flight += bytes;
    }

    /// The page acknowledged `acks` in one message, which arrived at `now`.
    pub fn acknowledged(&mut self, acks: &[ChunkAck], now: Instant) {
        let mut acked = Vec::with_capacity(acks.len());
        for ack in acks {
            // A chunk acknowledged twice, or of a stream before this one, is
            // nothing the path still holds.
            let Some(sent) = self.flight.remove(&(ack.generation, ack.sequence, ack.chunk)) else {
                continue;
            };
            self.in_flight -= sent.bytes;
            let round_trip = now.saturating_duration_since(sent.at);
            self.early = self
                .started
                .is_none_or(|started| now.saturating_duration_since(started) < ROUND_TRIP_SPAN);
            self.round_trips.push_back((now, round_trip));
            acked.push(AckedPacket {
                seq_no: sent.order.into(),
                size: DataSize::bytes(sent.bytes as i64),
                local_send_time: sent.at,
                remote_recv_time: self.page_time(ack.received, now),
                local_recv_time: now,
            });
        }
        while self
            .round_trips
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > ROUND_TRIP_SPAN)
        {
            self.round_trips.pop_front();
        }
        if !acked.is_empty() {
            self.estimate.update(acked, now);
        }
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

#[cfg(test)]
mod tests {
    //! The sender's rate on a simulated link: a bottleneck of a given
    //! capacity with a queue in front of it and a one-way delay each way,
    //! driven by frames that use the target and fit the window, as the
    //! viewer's delivery sends them. Each scenario simulates 30 to 40
    //! seconds in about a tenth of a second.

    use super::*;

    /// A link that carries `capacity` bits per second after a one-way `delay`.
    struct Link {
        capacity: f64,
        delay: Duration,
        /// When the bottleneck is free again.
        free: Instant,
        /// Acknowledgements on their way back: when they arrive, and what.
        acks: VecDeque<(Instant, ChunkAck)>,
    }

    impl Link {
        fn carry(&mut self, id: ChunkId, bytes: usize, now: Instant, origin: Instant) {
            let start = self.free.max(now);
            self.free = start + Duration::from_secs_f64(bytes as f64 * 8.0 / self.capacity);
            let received = self.free + self.delay;
            let ack = ChunkAck {
                generation: id.0,
                sequence: id.1,
                chunk: id.2,
                received: (received - origin).as_secs_f64() * 1000.0,
            };
            self.acks.push_back((received + self.delay, ack));
        }
    }

    struct Run {
        /// The target each second.
        targets: Vec<u32>,
        /// The frame rate and scale each second.
        ladder: Vec<(u32, f64)>,
        /// The longest a chunk waited in the bottleneck's queue.
        queued: Duration,
    }

    /// Sends 1080p frames for `seconds` over a link whose capacity
    /// `capacity` gives for each second.
    fn run(seconds: u32, delay: Duration, capacity: impl Fn(u32) -> f64) -> Run {
        run_at(1920 * 1080, seconds, delay, capacity)
    }

    /// Sends frames of `pixels` as [`run`] does.
    fn run_at(pixels: u64, seconds: u32, delay: Duration, capacity: impl Fn(u32) -> f64) -> Run {
        let origin = Instant::now();
        let mut rate = Rate::new(pixels);
        let mut link = Link {
            capacity: capacity(0),
            delay,
            free: origin,
            acks: VecDeque::new(),
        };
        let mut run = Run {
            targets: Vec::new(),
            ladder: Vec::new(),
            queued: Duration::ZERO,
        };
        let step = Duration::from_millis(1);
        let mut now = origin;
        let mut next_frame = origin;
        let mut sequence = 0;
        let mut pending: VecDeque<(Instant, ChunkId, usize)> = VecDeque::new();
        let mut batch = Vec::new();
        for millisecond in 0..seconds * 1000 {
            if millisecond % 1000 == 0 {
                link.capacity = capacity(millisecond / 1000);
                run.targets.push(rate.target());
                run.ladder.push((rate.fps(), rate.scale()));
            }
            // The page acknowledges a few chunks to a message.
            while link.acks.front().is_some_and(|(at, _)| *at <= now) {
                batch.push(link.acks.pop_front().unwrap().1);
                if batch.len() == 4 {
                    rate.acknowledged(&batch, now);
                    batch.clear();
                }
            }
            if !batch.is_empty() && link.acks.front().is_none_or(|(at, _)| *at > now + step) {
                rate.acknowledged(&batch, now);
                batch.clear();
            }
            // The encoder takes the target, and a frame waits for room.
            let interval = Duration::from_secs_f64(1.0 / f64::from(rate.fps()));
            let size = (f64::from(rate.target()) / 8.0 / f64::from(rate.fps())) as usize;
            let queued: usize = pending.iter().map(|(_, _, bytes)| bytes).sum();
            if now >= next_frame && rate.room(queued, size) {
                sequence += 1;
                let chunks = size.div_ceil(16 * 1024).max(1);
                let start = pending.back().map_or(now, |(at, _, _)| (*at).max(now));
                for chunk in 0..chunks {
                    let at = start + interval.mul_f64(chunk as f64 / chunks as f64);
                    let bytes = (size / chunks).max(1);
                    pending.push_back((at, (1, sequence, chunk as u16), bytes));
                }
                next_frame = now + interval;
            }
            while pending.front().is_some_and(|(at, _, _)| *at <= now) {
                let (_, id, bytes) = pending.pop_front().unwrap();
                rate.sent(id, bytes, now);
                link.carry(id, bytes, now, origin);
                run.queued = run.queued.max(link.free.saturating_duration_since(now));
            }
            now += step;
        }
        run
    }

    #[test]
    fn the_target_starts_at_five_megabits_and_climbs_to_what_the_link_carries() {
        let run = run(40, Duration::from_millis(25), |_| 20e6);
        assert_eq!(run.targets[0], INITIAL_BITRATE);
        let last = f64::from(*run.targets.last().unwrap());
        assert!(
            (12e6..=24e6).contains(&last),
            "the target ended at {last} on a 20 Mbps link: {:?}",
            run.targets
        );
        // The window holds the queue to a fraction of a second.
        assert!(run.queued < Duration::from_millis(400), "{:?} queued", run.queued);
    }

    #[test]
    fn a_drop_in_capacity_lowers_the_target_within_seconds_and_it_recovers() {
        let run = run(36, Duration::from_millis(10), |second| {
            if (16..24).contains(&second) { 5e6 } else { 40e6 }
        });
        let before = run.targets[15];
        let during = run.targets[19];
        assert!(before > 10_000_000, "the target before the drop: {:?}", run.targets);
        // The window keeps the queue from growing once the target fell, so
        // the estimate holds near what the link acknowledges, as GoogCC caps
        // it at 1.5 times the acknowledged rate.
        assert!(
            f64::from(during) <= f64::from(before) * 0.6 && during <= 8_000_000,
            "the target 3 s into the drop: {:?}",
            run.targets
        );
        assert!(run.queued < Duration::from_millis(600), "{:?} queued", run.queued);
        assert!(
            run.targets[35] > during,
            "the target after the drop: {:?}",
            run.targets
        );
    }

    #[test]
    fn the_window_is_the_target_over_the_least_recent_round_trip_and_at_least_a_frame() {
        let origin = Instant::now();
        let mut rate = Rate::new(1920 * 1080);
        // Before any round trip: the slack alone, 5 Mbps over 60 ms.
        assert_eq!(rate.window(), 37_500);
        assert!(rate.room(0, 1_000_000), "an idle path takes any frame");
        assert!(!rate.room(1, 1_000_000));
        let ack = |sequence, received| ChunkAck { generation: 1, sequence, chunk: 0, received };
        // A chunk resent by the path came back late; the least round trip wins.
        rate.sent((1, 1, 0), 1000, origin);
        rate.sent((1, 2, 0), 1000, origin + Duration::from_millis(10));
        rate.acknowledged(&[ack(2, 2.0)], origin + Duration::from_millis(50));
        rate.acknowledged(&[ack(1, 1.0)], origin + Duration::from_millis(900));
        assert_eq!(rate.round_trip(), Some(Duration::from_millis(40)));
        assert_eq!(rate.in_flight(), 0);
        // Ten seconds on, the old round trips are forgotten.
        rate.sent((1, 3, 0), 1000, origin + Duration::from_secs(11));
        rate.acknowledged(&[ack(3, 3.0)], origin + Duration::from_millis(11_200));
        assert_eq!(rate.round_trip(), Some(Duration::from_millis(200)));
    }

    /// The first chunk of a lossy path came back two seconds late, as a
    /// resent one does: in the view's first ten seconds the window counts
    /// 300 ms of round trip, not two seconds. Past them a steady long path,
    /// 450 ms, keeps its window whole. Planted defects this catches: the
    /// window taking the late sample whole; a cap that stays and starves a
    /// long path.
    #[test]
    fn the_window_counts_at_most_300_ms_early_in_a_view_and_a_long_path_whole_after() {
        let origin = Instant::now();
        let mut rate = Rate::new(1920 * 1080);
        let at = |milliseconds| origin + Duration::from_millis(milliseconds);
        let ack = |sequence, received| ChunkAck { generation: 1, sequence, chunk: 0, received };
        rate.sent((1, 1, 0), 1000, origin);
        rate.acknowledged(&[ack(1, 1.0)], at(2_000));
        // 5 Mbps over 300 + 60 ms.
        assert_eq!(rate.window(), 225_000);
        // A path of 450 ms, a chunk each half second, to 14 s.
        for sequence in 2..=28 {
            let sent = u64::from(sequence) * 500;
            rate.sent((1, sequence, 0), 1000, at(sent));
            rate.acknowledged(&[ack(sequence, f64::from(sequence))], at(sent + 450));
        }
        assert_eq!(rate.round_trip(), Some(Duration::from_millis(450)));
        let expected = (f64::from(rate.target()) / 8.0 * 0.51) as usize;
        assert_eq!(rate.window(), expected);
    }

    /// The relay's acceptance: a 20 Mbps link that falls to 5 and comes
    /// back. The frame rate falls with it and returns once the target is
    /// back where it was. Planted defect this catches: a return threshold so
    /// far above the fall's that a target near the link's capacity never
    /// reaches it, so the picture stays at the lowered rate.
    #[test]
    fn the_frame_rate_lowered_by_a_drop_returns_when_the_target_recovers() {
        // A 1440 × 900 panel at ratio 2.
        let run = run_at(2880 * 1800, 60, Duration::from_millis(25), |second| {
            if (20..30).contains(&second) { 5e6 } else { 20e6 }
        });
        let before = run.ladder[19];
        assert!(
            run.ladder[25].0 < before.0,
            "the drop lowered nothing: {:?} {:?}",
            run.ladder,
            run.targets
        );
        assert_eq!(run.ladder[59], before, "targets {:?}, ladder {:?}", run.targets, run.ladder);
    }

    #[test]
    fn a_low_target_lowers_the_frame_rate_then_the_resolution_and_a_high_one_raises_them_back() {
        // 2880 × 1800 device pixels: a 1440 × 900 panel at ratio 2.
        let mut rate = Rate::new(2880 * 1800);
        let ladder = |rate: &mut Rate, bitrate: u64| {
            rate.estimate = DelayBasedEstimate::new(Bitrate::bps(bitrate));
            rate.fit();
            (rate.fps(), rate.scale())
        };
        assert_eq!(ladder(&mut rate, 100_000_000), (60, 1.0));
        assert_eq!(ladder(&mut rate, 10_000_000), (30, 1.0));
        assert_eq!(ladder(&mut rate, 4_000_000), (15, 1.0));
        assert_eq!(ladder(&mut rate, 2_500_000), (15, 0.75));
        assert_eq!(ladder(&mut rate, 500_000), (15, 0.5));
        // Pixels come back first.
        assert_eq!(ladder(&mut rate, 3_500_000), (15, 0.75));
        assert_eq!(ladder(&mut rate, 5_000_000), (15, 1.0));
        assert_eq!(ladder(&mut rate, 10_000_000), (30, 1.0));
        assert_eq!(ladder(&mut rate, 20_000_000), (60, 1.0));
    }
}
