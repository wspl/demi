//! One viewer's pictures on their way to its page (`live-view.md`
//! § Delivery): which frames the page has, how many may be in flight, when
//! the next must be a key frame, and the encoding its path takes, which it
//! tells the stream of the source it watches.

use std::{collections::VecDeque, time::Duration};

use tokio::time::Instant;

use demi_live_view_protocol::{STALL_MS, SourceId};

use crate::{
    frames,
    rate::{Rate, Sample},
    stream::{EncodedFrame, StreamView},
    writer::{ModuleMessage, Writer},
};

/// The frames a viewer's page has, and the path they take.
pub struct Delivery {
    /// Counts the picture sequences the page was told about.
    generation: u32,
    /// The capture the current sequence comes from.
    epoch: u32,
    awaiting_key: bool,
    /// The newest capture sequence seen.
    last: u32,
    /// Frames the page has not acknowledged: sequence, when sent, bytes.
    flight: VecDeque<(u32, Instant, usize)>,
    rate: Rate,
    applied: Option<(u32, u32, f64)>,
    started: Instant,
    ticked: Instant,
    last_ack: Instant,
    round_trip: f64,
    decode_queue: u64,
    frames: u32,
    bytes: usize,
    acknowledged: usize,
    dropped: bool,
}

impl Default for Delivery {
    fn default() -> Self {
        Self::new()
    }
}

impl Delivery {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            generation: 0,
            epoch: 0,
            awaiting_key: true,
            last: 0,
            flight: VecDeque::new(),
            rate: Rate::new(1280 * 720),
            applied: None,
            started: now,
            ticked: now,
            last_ack: now,
            round_trip: 0.0,
            decode_queue: 0,
            frames: 0,
            bytes: 0,
            acknowledged: 0,
            dropped: false,
        }
    }

    /// The bit rate, frame rate and scale the viewer's path takes, once
    /// measured; a stream the viewer starts watching takes them too.
    pub fn applied(&self) -> Option<(u32, u32, f64)> {
        self.applied
    }

    /// The viewer watches another source, or none: its pictures start over
    /// from a key frame.
    pub fn rewatch(&mut self) {
        self.awaiting_key = true;
        self.flight.clear();
        self.last = 0;
    }

    /// Pictures of a new generation of `width` × `height` pixels follow,
    /// from the capture `epoch`; answers the generation's number for the
    /// page.
    pub fn restart(&mut self, epoch: u32, width: u32, height: u32) -> u32 {
        self.generation += 1;
        self.epoch = epoch;
        self.awaiting_key = true;
        self.last = 0;
        self.flight.clear();
        self.rate.resize(u64::from(width) * u64::from(height));
        self.generation
    }

    /// Sends `frame` of `source` to the page when the path takes it and the
    /// page can decode it, and tells the stream how far the page is.
    pub fn frame<M: ModuleMessage, L>(
        &mut self,
        source: &SourceId,
        frame: &EncodedFrame,
        writer: &Writer<M>,
        stream: Option<&StreamView<L>>,
    ) {
        if frame.sequence <= self.last {
            return;
        }
        // A frame the viewer never had breaks the pictures after it.
        if self.last != 0 && frame.sequence != self.last + 1 && !frame.key {
            self.awaiting_key = true;
            if let Some(stream) = stream {
                stream.key_frame();
            }
        }
        self.last = frame.sequence;
        if self.awaiting_key && !frame.key {
            self.pace(stream);
            return;
        }
        let bytes = frames::video(source, self.generation, frame.sequence, frame);
        let length = bytes.len();
        if writer.video(bytes) {
            self.awaiting_key = false;
            self.flight
                .push_back((frame.sequence, Instant::now(), length));
            self.frames += 1;
            self.bytes += length;
        } else {
            self.dropped = true;
            self.awaiting_key = true;
            if let Some(stream) = stream {
                stream.key_frame();
            }
        }
        self.pace(stream);
    }

    /// The page's decoder lost the pictures of `generation`.
    pub fn key_frame<L>(&mut self, generation: u32, stream: Option<&StreamView<L>>) {
        if self.generation == generation {
            self.awaiting_key = true;
            if let Some(stream) = stream {
                stream.key_frame();
            }
        }
    }

    /// The page showed `sequence` of `generation`, with `decode_queue`
    /// frames waiting for its decoder.
    pub fn acknowledged<L>(
        &mut self,
        generation: u32,
        sequence: u32,
        decode_queue: u32,
        stream: Option<&StreamView<L>>,
    ) {
        if self.generation != generation {
            return;
        }
        let now = Instant::now();
        while let Some(&(sent, at, length)) = self.flight.front()
            && sent <= sequence
        {
            self.flight.pop_front();
            self.acknowledged += length;
            if sent == sequence {
                let round_trip = (now - at).as_secs_f64() * 1000.0;
                self.rate.acknowledged(round_trip);
                self.round_trip = round_trip;
            }
        }
        self.decode_queue = u64::from(decode_queue);
        self.last_ack = now;
        self.pace(stream);
    }

    /// Adapts the picture to the path; the viewer calls it once a second.
    /// `queued` is the video bytes its writer has not written yet.
    pub fn tick<L>(&mut self, queued: usize, stream: Option<&StreamView<L>>) {
        let now = Instant::now();
        let elapsed = now - self.ticked;
        self.ticked = now;
        let stall = Duration::from_millis(STALL_MS);
        let oldest = self.flight.front().map(|(_, sent, _)| *sent);
        // A Host pause, or a page that acknowledges nothing, is a stall, not
        // congestion: the budget stays. Frames the page has not acknowledged
        // for long never hold back the capture; the page resumes from a key
        // frame.
        let stalled = elapsed > 3 * Duration::from_secs(1)
            || oldest.is_some_and(|sent| now - sent > stall && self.last_ack < sent);
        if stalled {
            if oldest.is_some_and(|sent| now - sent > 2 * stall) {
                self.flight.clear();
                self.awaiting_key = true;
                self.pace(stream);
            }
        } else {
            let seconds = elapsed.as_secs_f64().max(0.001);
            let sample = Sample {
                now: (now - self.started).as_secs_f64() * 1000.0,
                buffered_bytes: queued,
                ack_age: oldest.map_or(0.0, |sent| (now - sent).as_secs_f64() * 1000.0),
                round_trip: if self.frames > 0 { self.round_trip } else { 0.0 },
                decode_queue: self.decode_queue,
                active_frames: self.frames,
                congested: self.dropped,
                encoded_bitrate: self.bytes as f64 * 8.0 / seconds,
                delivered_bitrate: self.acknowledged as f64 * 8.0 / seconds,
            };
            if let Some(cause) = self.rate.update(&sample) {
                // Why a picture lost quality is found in the Host's log, not guessed.
                tracing::info!(
                    "live view rate fell to {} bps, {} fps, scale {} after {cause}: {} bytes waiting, oldest frame {:.0} ms, round trip {:.0} ms, decode queue {}, sent {:.0} bps, acknowledged {:.0} bps",
                    self.rate.bitrate(),
                    self.rate.fps(),
                    self.rate.scale(),
                    sample.buffered_bytes,
                    sample.ack_age,
                    sample.round_trip,
                    sample.decode_queue,
                    sample.encoded_bitrate,
                    sample.delivered_bitrate,
                );
            }
            let encoding = (self.rate.bitrate(), self.rate.fps(), self.rate.scale());
            if self.applied != Some(encoding) {
                self.applied = Some(encoding);
                if let Some(stream) = stream {
                    stream.encoding(encoding.0, encoding.1, encoding.2);
                }
            }
        }
        self.frames = 0;
        self.bytes = 0;
        self.acknowledged = 0;
        self.dropped = false;
    }

    /// Frames up to the floor are shown or will never be: the capture may
    /// run `window` frames past it.
    fn floor(&self) -> u32 {
        self.flight
            .front()
            .map_or(self.last, |(sequence, _, _)| sequence.saturating_sub(1))
    }

    fn pace<L>(&self, stream: Option<&StreamView<L>>) {
        if let Some(stream) = stream {
            stream.pace(self.epoch, self.floor(), self.rate.window);
        }
    }
}
