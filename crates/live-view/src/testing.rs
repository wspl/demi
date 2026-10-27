//! A simulated path for the rate control's tests (`tests/live-view`): the
//! Host's sender as `delivery` drives the rate control, a link with a capacity
//! and a queue, and a page that stamps each chunk when its main thread takes
//! it, as the web app does (`packages/web-ui/src/live/link.ts`).
//!
//! The run takes no time: it steps a simulated clock a millisecond at a time.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use demi_live_view_protocol::{CHUNK_BYTES, ChunkAck};

use crate::rate::{ChunkId, Rate};

/// A picture of an iPhone's screen, as the panel shows it at full size.
const PIXELS: u64 = 1206 * 2622;
/// How often the run looks at the rate control.
const SAMPLE: Duration = Duration::from_millis(10);
const STEP: Duration = Duration::from_millis(1);

/// What the run simulates.
pub struct Scenario {
    /// The bytes of the picture coded at a time; 0 while it stands still.
    pub picture: Box<dyn Fn(Duration) -> usize>,
    /// The link's capacity in bits a second from each time on; none, or
    /// before the first, is a loopback that queues nothing.
    pub capacity: Vec<(Duration, f64)>,
    /// When the page falls behind, for how long, and how much later it takes
    /// each chunk for each second of it: its main thread busy, as a layout
    /// keeps it, it takes what arrives later and later, then catches up.
    pub page_behind: Option<(Duration, Duration, f64)>,
    /// How long the run lasts.
    pub until: Duration,
}

/// The rate control as it stood at a time.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub at: Duration,
    /// The bit rate the encoder is asked for.
    pub target: u32,
    pub fps: u32,
    pub scale: f64,
    /// How long a chunk sent then waits in the link's queue.
    pub queue: Duration,
}

/// Runs `scenario`, answering a sample every 10 ms.
pub fn run(scenario: &Scenario) -> Vec<Sample> {
    let origin = Instant::now();
    let mut rate = Rate::new(PIXELS);
    let mut paced: VecDeque<(Duration, ChunkId, usize, bool)> = VecDeque::new();
    // Chunks on their way: when the page takes each.
    let mut arriving: Vec<(Duration, ChunkId)> = Vec::new();
    // When the link is free to send the next bit.
    let mut link_free = Duration::ZERO;
    let mut next_frame = Duration::ZERO;
    let mut sequence = 0u32;
    let mut samples = Vec::new();
    let mut next_sample = Duration::ZERO;
    // When the page's main thread took the last chunk: it takes them in the
    // order they arrived.
    let mut page_free = Duration::ZERO;
    let mut now = Duration::ZERO;
    while now < scenario.until {
        if now >= next_frame {
            next_frame += Duration::from_secs(1) / rate.fps();
            let bytes = (scenario.picture)(now);
            if bytes > 0 && rate.room(0, bytes) {
                sequence += 1;
                let chunks = bytes.div_ceil(CHUNK_BYTES);
                for chunk in 0..chunks {
                    let id = (1, sequence, chunk as u16, false);
                    paced.push_back((now, id, bytes.div_ceil(chunks), false));
                }
            }
        }
        while let Some(probe) = rate.next_probe() {
            let bytes = probe.bytes.as_bytes_usize() / probe.chunks;
            let spacing = Duration::from_secs_f64(bytes as f64 * 8.0 / probe.target.as_f64().max(1.0));
            let start = paced.back().map_or(now, |last| last.0.max(now));
            for index in 0..probe.chunks {
                let id = (1, probe.id, index as u16, true);
                paced.push_back((start + spacing * index as u32, id, bytes, index + 1 == probe.chunks));
            }
        }
        while paced.front().is_some_and(|chunk| chunk.0 <= now) {
            let (_, id, bytes, ends_probe) = paced.pop_front().expect("a chunk is due");
            rate.sent(id, bytes, origin + now);
            if ends_probe {
                rate.probe_sent(id.1, origin + now);
            }
            let arrives = match capacity(scenario, now) {
                Some(bps) => {
                    link_free = link_free.max(now) + Duration::from_secs_f64(bytes as f64 * 8.0 / bps);
                    link_free
                }
                None => now,
            };
            page_free = page_free.max(arrives + behind(scenario, arrives));
            arriving.push((page_free, id));
        }
        // The page acknowledges what its main thread took, in one message.
        let (taken_now, later): (Vec<_>, Vec<_>) = arriving.into_iter().partition(|(at, _)| *at <= now);
        arriving = later;
        if !taken_now.is_empty() {
            let acks: Vec<ChunkAck> = taken_now
                .iter()
                .map(|(at, (generation, sequence, chunk, padding))| ChunkAck {
                    generation: *generation,
                    sequence: *sequence,
                    chunk: *chunk,
                    padding: *padding,
                    received: at.as_secs_f64() * 1000.0,
                })
                .collect();
            rate.acknowledged(&acks, origin + now);
        }
        if now >= next_sample {
            next_sample += SAMPLE;
            samples.push(Sample {
                at: now,
                target: rate.target(),
                fps: rate.fps(),
                scale: rate.scale(),
                queue: link_free.saturating_sub(now),
            });
        }
        now += STEP;
    }
    samples
}

fn capacity(scenario: &Scenario, now: Duration) -> Option<f64> {
    scenario.capacity.iter().rev().find(|(from, _)| now >= *from).map(|(_, bps)| *bps)
}

/// How long after it arrives at `at` the page's main thread is free for a
/// chunk; it then takes the chunks waiting in their order.
fn behind(scenario: &Scenario, at: Duration) -> Duration {
    match scenario.page_behind {
        Some((from, length, lost)) if at >= from && at < from + length => (at - from).mul_f64(lost),
        _ => Duration::ZERO,
    }
}
