//! A stream's last entries, kept within a count and a size, and read after
//! a cursor: a browser tab's console and debugging events, a device's log.
//!
//! Each entry takes the next position of its stream. A cursor names a
//! position of one buffer's stream, its generation, so a cursor of a buffer
//! that was replaced, such as a device's after it was opened again, is
//! refused rather than read as a position of the new one.

use std::collections::VecDeque;

use serde::Serialize;

/// An entry whose position in its stream the buffer assigns.
pub trait Entry: Serialize {
    fn sequence(&self) -> u64;
    fn set_sequence(&mut self, sequence: u64);
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BufferError {
    /// The cursor names another buffer, or a position past the stream's end.
    #[error("the cursor is not one of this stream's")]
    StaleCursor,
    #[error("an entry does not serialize: {0}")]
    Unserializable(String),
    /// The stream used every position.
    #[error("the stream has no more positions")]
    Exhausted,
}

pub struct Buffer<T> {
    generation: String,
    next: u64,
    bytes: usize,
    entry_limit: usize,
    byte_limit: usize,
    evicted_through: Option<u64>,
    entries: VecDeque<(T, usize)>,
}

impl<T: Entry> Buffer<T> {
    /// An empty buffer whose cursors start with `generation`, which no
    /// other buffer a reader may hold a cursor of has.
    pub fn new(generation: String, entry_limit: usize, byte_limit: usize) -> Self {
        Self {
            generation,
            next: 0,
            bytes: 0,
            entry_limit,
            byte_limit,
            evicted_through: None,
            entries: VecDeque::new(),
        }
    }

    /// Appends an entry at the next position, evicting the oldest past the
    /// limits. Its size is its JSON's, as a reader receives it.
    pub fn push(&mut self, mut entry: T) -> Result<(), BufferError> {
        entry.set_sequence(self.next);
        let size = serde_json::to_vec(&entry)
            .map_err(|error| BufferError::Unserializable(error.to_string()))?
            .len();
        self.next = self.next.checked_add(1).ok_or(BufferError::Exhausted)?;
        self.bytes += size;
        self.entries.push_back((entry, size));
        while self.entries.len() > self.entry_limit || self.bytes > self.byte_limit {
            if let Some((entry, size)) = self.entries.pop_front() {
                self.bytes -= size;
                let sequence = entry.sequence();
                self.evicted_through = Some(self.evicted_through.map_or(sequence, |previous| previous.max(sequence)));
            }
        }
        Ok(())
    }

    /// Reserves one missing position when the stream's source lost data.
    pub fn mark_gap(&mut self) -> Result<(), BufferError> {
        let next = self.next.checked_add(1).ok_or(BufferError::Exhausted)?;
        self.evicted_through = Some(self.next);
        self.next = next;
        Ok(())
    }

    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.entries.iter().map(|(entry, _)| entry)
    }

    /// The position the next entry takes.
    pub fn next(&self) -> u64 {
        self.next
    }

    pub fn cursor(&self, position: u64) -> String {
        format!("{}:{position}", self.generation)
    }

    /// The position `cursor` names.
    pub fn position(&self, cursor: &str) -> Result<u64, BufferError> {
        let (generation, position) = cursor.rsplit_once(':').ok_or(BufferError::StaleCursor)?;
        let position = position.parse::<u64>().map_err(|_| BufferError::StaleCursor)?;
        if generation != self.generation || position > self.next {
            return Err(BufferError::StaleCursor);
        }
        Ok(position)
    }

    /// Whether an entry at or after `position` is gone.
    pub fn truncated_since(&self, position: u64) -> bool {
        self.evicted_through.is_some_and(|sequence| position <= sequence)
    }

    /// How many entries at or after `position` are gone, evicted or lost.
    pub fn dropped_since(&self, position: u64) -> u64 {
        let kept = self.entries().rev().take_while(|entry| entry.sequence() >= position).count() as u64;
        self.next.saturating_sub(position).saturating_sub(kept)
    }

    pub fn has_evicted(&self) -> bool {
        self.evicted_through.is_some()
    }
}
