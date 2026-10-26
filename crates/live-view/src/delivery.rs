//! One viewer's pictures on their way to its page: each frame in chunks spread
//! over the frame's interval, the window that holds what is unacknowledged,
//! when the next frame must be a key frame, and the encoding its path takes,
//! which it tells the stream of the source it watches.
//!
//! The capture encodes a picture only when the window has room for it. The
//! delivery tells the stream how far the page's frames reach and whether one
//! more may come, and the stream takes the least of its viewers'. An encoded
//! frame is never dropped, since later frames are coded against it.
//!
//! The rate control's probes (`probe`) go between the pictures: padding
//! chunks of the viewer's generation, spread over the probe's span at its
//! rate.

use std::{collections::VecDeque, time::Duration};

use bytes::Bytes;
use tokio::time::Instant;

use demi_live_view_protocol::{ChunkAck, SourceId};

use crate::{
    frames,
    rate::{ChunkId, Rate},
    stream::{EncodedFrame, StreamView},
    writer::{ModuleMessage, Writer},
};

/// A chunk waiting for its time.
struct Paced {
    at: Instant,
    id: ChunkId,
    bytes: Bytes,
    /// The last chunk of the probe it is padding of.
    ends_probe: bool,
}

/// The frames a viewer's page has, and the path they take.
pub struct Delivery {
    /// Counts the picture sequences the page was told about.
    generation: u32,
    /// The capture the current sequence comes from.
    epoch: u32,
    awaiting_key: bool,
    /// The newest capture sequence seen.
    last: u32,
    rate: Rate,
    /// Chunks waiting for their time, in order.
    paced: VecDeque<Paced>,
    /// The bytes of `paced`.
    queued: usize,
    /// The last frame's bytes: the guess at the next one's.
    frame_bytes: usize,
    applied: Option<(u32, u32, f64)>,
    /// The pace the stream was last told.
    told: Option<(u32, u32, u32)>,
    /// The source of the pictures, which a probe's padding names too.
    source: Option<SourceId>,
    /// When the delivery began, and its first picture's way to the page,
    /// which one line reports once the page acknowledged it.
    began: Instant,
    first: Option<FirstFrame>,
}

/// The first picture a delivery sent: its generation and sequence, when it
/// was queued, and whether its line was written.
struct FirstFrame {
    generation: u32,
    sequence: u32,
    queued: Instant,
    written: bool,
}

impl Default for Delivery {
    fn default() -> Self {
        Self::new()
    }
}

impl Delivery {
    pub fn new() -> Self {
        Self {
            generation: 0,
            epoch: 0,
            awaiting_key: true,
            last: 0,
            rate: Rate::new(1280 * 720),
            paced: VecDeque::new(),
            queued: 0,
            frame_bytes: 0,
            applied: None,
            told: None,
            source: None,
            began: Instant::now(),
            first: None,
        }
    }

    /// The bit rate, frame rate and scale the viewer's path takes, once
    /// known; a stream the viewer starts watching takes them too.
    pub fn applied(&self) -> Option<(u32, u32, f64)> {
        self.applied
    }

    /// The viewer watches another source, or none: its pictures start over
    /// from a key frame, and the chunks of the one before that wait for
    /// their time are not sent.
    pub fn rewatch(&mut self) {
        self.awaiting_key = true;
        self.last = 0;
        self.paced.clear();
        self.queued = 0;
        self.told = None;
    }

    /// Pictures of a new generation of `width` × `height` pixels at `scale`
    /// of the source's follow, from the capture `epoch`, after its frame
    /// `sequence`; answers the generation's number for the page.
    pub fn restart<L>(
        &mut self,
        epoch: u32,
        width: u32,
        height: u32,
        scale: f64,
        sequence: u32,
        stream: Option<&StreamView<L>>,
    ) -> u32 {
        self.generation += 1;
        self.epoch = epoch;
        self.rewatch();
        self.rate.restart_if_stale(Instant::now().into_std());
        // The capture runs on for the other viewers: this one's pictures
        // start where it is, so its pace never holds the capture back.
        self.last = sequence;
        // The ladder weighs the picture at full resolution.
        let pixels = f64::from(width) * f64::from(height) / (scale * scale).max(0.01);
        self.rate.resize(pixels as u64);
        self.encode(stream);
        self.pace(stream);
        self.generation
    }

    /// Queues `frame` of `source`'s chunks for the page, spread over the
    /// frame's interval, when the page can decode it, and tells the stream
    /// how far the page is.
    pub fn frame<L>(&mut self, source: &SourceId, frame: &EncodedFrame, stream: Option<&StreamView<L>>) {
        if frame.sequence <= self.last {
            return;
        }
        self.source = Some(source.clone());
        // A frame the viewer never had breaks the pictures after it.
        if self.last != 0 && frame.sequence != self.last + 1 && !frame.key {
            self.lost(stream);
        }
        self.last = frame.sequence;
        if self.awaiting_key && !frame.key {
            self.pace(stream);
            return;
        }
        let Some(chunks) = frames::chunks(source, self.generation, frame) else {
            tracing::warn!(event = "live_view_frame.refused", bytes = frame.data.len(), reason = "too_many_chunks", "a live view frame was refused");
            self.lost(stream);
            self.pace(stream);
            return;
        };
        self.awaiting_key = false;
        let now = Instant::now();
        self.first.get_or_insert(FirstFrame {
            generation: self.generation,
            sequence: frame.sequence,
            queued: now,
            written: false,
        });
        let interval = std::time::Duration::from_secs_f64(1.0 / f64::from(self.rate.fps()));
        // A frame's chunks follow the ones before them.
        let start = self.paced.back().map_or(now, |paced| paced.at.max(now));
        let count = chunks.len() as u32;
        self.frame_bytes = 0;
        for (index, chunk) in (0..).zip(chunks) {
            let at = start + interval * u32::from(index) / count;
            self.frame_bytes += chunk.len();
            self.queued += chunk.len();
            self.paced.push_back(Paced {
                at,
                id: (self.generation, frame.sequence, index, false),
                bytes: chunk,
                ends_probe: false,
            });
        }
        self.pace(stream);
    }

    /// Queues the probes the rate control planned after what waits, each
    /// chunk at its time for the probe's rate.
    fn probe(&mut self) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let now = Instant::now();
        while let Some(probe) = self.rate.next_probe() {
            let chunks = u16::try_from(probe.chunks).unwrap_or(u16::MAX);
            let bytes = probe.bytes.as_bytes_usize() / usize::from(chunks);
            let spacing = Duration::from_secs_f64(bytes as f64 * 8.0 / probe.target.as_f64().max(1.0));
            let start = self.paced.back().map_or(now, |paced| paced.at.max(now));
            for index in 0..chunks {
                let chunk = frames::padding(&source, self.generation, probe.id, index, chunks, bytes);
                self.queued += chunk.len();
                self.paced.push_back(Paced {
                    at: start + spacing * u32::from(index),
                    id: (self.generation, probe.id, index, true),
                    bytes: chunk,
                    ends_probe: index + 1 == chunks,
                });
            }
        }
    }

    /// When the next chunk is due; none while none waits.
    pub fn due(&self) -> Option<Instant> {
        self.paced.front().map(|paced| paced.at)
    }

    /// Hands the chunks that are due to `writer`.
    pub fn release<M: ModuleMessage>(&mut self, writer: &Writer<M>) {
        let now = Instant::now();
        while let Some(paced) = self.paced.front()
            && paced.at <= now
        {
            let paced = self.paced.pop_front().expect("a chunk is due");
            self.queued -= paced.bytes.len();
            self.rate.sent(paced.id, paced.bytes.len(), now.into_std());
            if paced.ends_probe {
                self.rate.probe_sent(paced.id.1, now.into_std());
            }
            writer.video(paced.bytes);
        }
        self.probe();
    }

    /// The page's decoder could not decode a frame of `generation`.
    pub fn key_frame<L>(&mut self, generation: u32, stream: Option<&StreamView<L>>) {
        if self.generation == generation {
            self.lost(stream);
            self.pace(stream);
        }
    }

    /// The page acknowledged `acks` in one message.
    pub fn acknowledged<L>(&mut self, acks: &[ChunkAck], stream: Option<&StreamView<L>>) {
        if let Some(first) = &mut self.first
            && !first.written
            && acks.iter().any(|ack| {
                !ack.padding && ack.generation == first.generation && ack.sequence == first.sequence
            })
        {
            first.written = true;
            tracing::info!(
                event = "live_view.first_acked",
                queued_ms = first.queued.duration_since(self.began).as_millis() as u64,
                acked_ms = self.began.elapsed().as_millis() as u64,
                "a viewer's page acknowledged its first picture"
            );
        }
        self.rate.acknowledged(acks, Instant::now().into_std());
        self.probe();
        self.encode(stream);
        self.pace(stream);
    }

    /// The next frame must be a key frame, which the stream is asked for.
    fn lost<L>(&mut self, stream: Option<&StreamView<L>>) {
        self.awaiting_key = true;
        if let Some(stream) = stream {
            stream.key_frame();
        }
    }

    /// Tells the stream the encoding the path takes when it changed enough
    /// to matter.
    fn encode<L>(&mut self, stream: Option<&StreamView<L>>) {
        let encoding = (self.rate.target(), self.rate.fps(), self.rate.scale());
        let changed = match self.applied {
            None => true,
            Some((bitrate, fps, scale)) => {
                fps != encoding.1 || scale != encoding.2 || self.rate.retarget(bitrate)
            }
        };
        if !changed {
            return;
        }
        // Why a picture changed quality is found in the Host's log, not guessed.
        tracing::info!(
            event = "live_view.rate_changed",
            bps = encoding.0,
            fps = encoding.1,
            scale = encoding.2,
            rtt_ms = self.rate.round_trip().map(|rtt| rtt.as_millis() as u64),
            window_bytes = self.rate.window(),
            in_flight = self.rate.in_flight(),
            "a live view's encoding changed"
        );
        self.applied = Some(encoding);
        if let Some(stream) = stream {
            stream.encoding(encoding.0, encoding.1, encoding.2);
        }
    }

    /// Tells the stream how far the page's frames reach and whether the
    /// window has room for one more.
    fn pace<L>(&mut self, stream: Option<&StreamView<L>>) {
        let room = self.awaiting_key || self.rate.room(self.queued, self.frame_bytes);
        let pace = (self.epoch, self.last, u32::from(room));
        if self.told == Some(pace) {
            return;
        }
        if let Some(stream) = stream {
            self.told = Some(pace);
            stream.pace(pace.0, pace.1, pace.2);
        }
    }
}
