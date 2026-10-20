//! One viewer's pictures on their way to its page: each frame in chunks spread
//! over the frame's interval, the window that holds what is unacknowledged,
//! when the next frame must be a key frame, and the encoding its path takes,
//! which it tells the stream of the source it watches.
//!
//! The capture encodes a picture only when the window has room for it: the
//! delivery tells the stream how far the page's frames reach and whether one
//! more may come, and the stream takes the least of its viewers'. An encoded
//! frame is never dropped, since later frames are coded against it.

use std::collections::VecDeque;

use bytes::Bytes;
use tokio::time::Instant;

use demi_live_view_protocol::{ChunkAck, SourceId};

use crate::{
    frames,
    rate::{ChunkId, Rate},
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
    rate: Rate,
    /// Chunks waiting for their time, in order: when, which, the bytes.
    paced: VecDeque<(Instant, ChunkId, Bytes)>,
    /// The bytes of `paced`.
    queued: usize,
    /// The last frame's bytes: the guess at the next one's.
    frame_bytes: usize,
    applied: Option<(u32, u32, f64)>,
    /// The pace the stream was last told.
    told: Option<(u32, u32, u32)>,
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
            tracing::warn!("live view frame of {} bytes has too many chunks", frame.data.len());
            self.lost(stream);
            self.pace(stream);
            return;
        };
        self.awaiting_key = false;
        let now = Instant::now();
        let interval = std::time::Duration::from_secs_f64(1.0 / f64::from(self.rate.fps()));
        // A frame's chunks follow the ones before them.
        let start = self.paced.back().map_or(now, |(at, _, _)| (*at).max(now));
        let count = chunks.len() as u32;
        self.frame_bytes = 0;
        for (index, chunk) in (0..).zip(chunks) {
            let at = start + interval * u32::from(index) / count;
            self.frame_bytes += chunk.len();
            self.queued += chunk.len();
            self.paced
                .push_back((at, (self.generation, frame.sequence, index), chunk));
        }
        self.pace(stream);
    }

    /// When the next chunk is due; none while none waits.
    pub fn due(&self) -> Option<Instant> {
        self.paced.front().map(|(at, _, _)| *at)
    }

    /// Hands the chunks that are due to `writer`.
    pub fn release<M: ModuleMessage>(&mut self, writer: &Writer<M>) {
        let now = Instant::now();
        while let Some((at, _, _)) = self.paced.front()
            && *at <= now
        {
            let (_, id, chunk) = self.paced.pop_front().expect("a chunk is due");
            self.queued -= chunk.len();
            self.rate.sent(id, chunk.len(), now.into_std());
            writer.video(chunk);
        }
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
        self.rate.acknowledged(acks, Instant::now().into_std());
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
        tracing::debug!(
            "live view encoding {} bps, {} fps, scale {}: least round trip {:?}, window {} bytes, {} in flight",
            encoding.0,
            encoding.1,
            encoding.2,
            self.rate.round_trip(),
            self.rate.window(),
            self.rate.in_flight(),
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
