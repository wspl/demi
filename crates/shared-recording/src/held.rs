//! What a source holds for the pictures its encoder has not answered yet: where
//! each differs from the one before, until the encoder gives it back or says it
//! dropped it.
//!
//! A hardware encoder works behind its calls, so a picture's place in the video
//! is known only when the encoder answers. One it drops is counted as missed,
//! and the picture the encoder gives next differs from the one before the
//! dropped, in its own area and in the dropped ones': the video has no frame in
//! between.

use std::collections::VecDeque;

use demi_shared_recording_protocol::Area;

/// What the encoder's answer settles about a picture, in the order they were
/// drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The encoder gave the picture drawn at `at`; it differs from the picture
    /// before in `changed`.
    Given { at: u64, changed: Area },
    /// The encoder dropped a picture, which is counted as missed.
    Dropped,
}

/// The pictures given to an encoder and not yet answered.
#[derive(Debug, Default)]
pub struct Held {
    /// Each picture's time and the area it differs in from the picture drawn
    /// before it, in the order given.
    waiting: VecDeque<(u64, Area)>,
    /// Where the pictures dropped since the last one given differ.
    dropped: Option<Area>,
}

impl Held {
    /// A picture drawn at `at` was given to the encoder.
    pub fn give(&mut self, at: u64, changed: Area) {
        self.waiting.push_back((at, changed));
    }

    /// The encoder gave the pictures drawn at the times `given` and dropped
    /// those drawn at `dropped`: what that settles, oldest first. A picture
    /// the encoder has not answered stays held, with every later one.
    pub fn settle(&mut self, given: &[u64], dropped: &[u64]) -> Vec<Settled> {
        let mut settled = Vec::new();
        while let Some(&(at, changed)) = self.waiting.front() {
            if dropped.contains(&at) {
                self.dropped = Some(self.dropped.map_or(changed, |before| before.union(changed)));
                settled.push(Settled::Dropped);
            } else if given.contains(&at) {
                let changed = self.dropped.take().map_or(changed, |before| before.union(changed));
                settled.push(Settled::Given { at, changed });
            } else {
                break;
            }
            self.waiting.pop_front();
        }
        settled
    }

    /// The encoder ended with every picture answered or lost: nothing is held.
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.dropped = None;
    }
}
