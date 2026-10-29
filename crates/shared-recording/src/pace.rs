//! How a video with `--fast-waits` plays a recording's time: a wait, a
//! stretch in which no frame changed and no input came, plays faster.
//!
//! ```text
//! recording  |--f--i------------------------f--|   a 7 s wait
//!                  |settle|  8x faster |lead|
//! video      |--f--i------|==|----f--|
//! ```
//!
//! The first moment of a wait plays as it was, so what changed last is
//! seen, and the moment before the next change too, so it is seen coming.
//! Only the video's time moves; every time a listing or an answer names
//! stays the recording's own.

use crate::{caption::FAST, stored::Contents};

/// How long a stretch with nothing in it must be to play faster.
const QUIET_US: u64 = 2_000_000;
/// How much of a wait plays as it was, after the change before it.
const SETTLE_US: u64 = 1_000_000;
/// And before the change after it.
const LEAD_US: u64 = 500_000;

/// The stretches of one recording that play faster, in its own time.
pub(crate) struct Pace {
    fast: Vec<(u64, u64)>,
}

impl Pace {
    /// Every moment as it was.
    pub(crate) fn even() -> Self {
        Self { fast: Vec::new() }
    }

    /// The waits of the recording `contents`, which ran `ran` microseconds.
    pub(crate) fn of(contents: &Contents, ran: u64) -> Self {
        let mut changes: Vec<u64> = contents
            .frames
            .iter()
            .map(|kept| kept.t)
            .chain(contents.inputs.iter().map(|row| row.time_us))
            .chain([0, ran])
            .filter(|t| *t <= ran)
            .collect();
        changes.sort_unstable();
        changes.dedup();
        let fast = changes
            .windows(2)
            .filter(|pair| pair[1] - pair[0] > QUIET_US)
            .map(|pair| (pair[0] + SETTLE_US, pair[1] - LEAD_US))
            .collect();
        Self { fast }
    }

    /// Whether any of it plays faster.
    pub(crate) fn is_even(&self) -> bool {
        self.fast.is_empty()
    }

    /// When the video plays the moment `t` of the recording, from its start.
    pub(crate) fn plays(&self, t: u64) -> u64 {
        let saved: u64 = self
            .fast
            .iter()
            .map(|(from, until)| (t.min(*until)).saturating_sub(*from))
            .map(|faster| faster - faster / FAST)
            .sum();
        t - saved
    }

    /// Whether the moment `t` plays faster.
    pub(crate) fn is_fast(&self, t: u64) -> bool {
        self.fast.iter().any(|(from, until)| (*from..*until).contains(&t))
    }

    /// Where the stretches that play faster begin and end, at which the
    /// badge shows and goes.
    pub(crate) fn bounds(&self) -> impl Iterator<Item = u64> + '_ {
        self.fast.iter().flat_map(|(from, until)| [*from, *until])
    }
}
