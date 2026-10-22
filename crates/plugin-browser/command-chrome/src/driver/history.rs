//! Browser console and CDP streams share bounded retention and generation cursors.
use std::collections::VecDeque;

use serde::Serialize;

use crate::driver::{
    operation::{BrowserError, Result},
    protocol::{CdpEvent, LogEntry},
};

/// An entry whose position in its stream the buffer assigns.
pub trait Entry: Serialize {
    fn sequence(&self) -> u64;
    fn set_sequence(&mut self, sequence: u64);
}

impl Entry for LogEntry {
    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn set_sequence(&mut self, sequence: u64) {
        self.sequence = sequence;
    }
}

impl Entry for CdpEvent {
    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn set_sequence(&mut self, sequence: u64) {
        self.sequence = sequence;
    }
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
    pub fn new(prefix: &str, entry_limit: usize, byte_limit: usize) -> Result<Self> {
        Ok(Self {
            generation: crate::driver::handles::fresh(prefix)?,
            next: 0,
            bytes: 0,
            entry_limit,
            byte_limit,
            evicted_through: None,
            entries: VecDeque::new(),
        })
    }

    /// Appends an entry at the next position, evicting the oldest past the limits.
    pub fn push(&mut self, mut entry: T) -> Result<()> {
        entry.set_sequence(self.next);
        let size = serde_json::to_vec(&entry)
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
            .len();
        self.next = self
            .next
            .checked_add(1)
            .ok_or(BrowserError::ResultTooLarge)?;
        self.bytes += size;
        self.entries.push_back((entry, size));
        while self.entries.len() > self.entry_limit || self.bytes > self.byte_limit {
            if let Some((entry, size)) = self.entries.pop_front() {
                self.bytes -= size;
                let sequence = entry.sequence();
                self.evicted_through = Some(
                    self.evicted_through
                        .map_or(sequence, |previous| previous.max(sequence)),
                );
            }
        }
        Ok(())
    }

    /// Reserve one missing browser event position when the upstream listener loses data.
    pub fn mark_gap(&mut self) -> Result<()> {
        let next = self
            .next
            .checked_add(1)
            .ok_or(BrowserError::ResultTooLarge)?;
        self.evicted_through = Some(self.next);
        self.next = next;
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|(entry, _)| entry)
    }
    pub fn next(&self) -> u64 {
        self.next
    }
    pub fn cursor(&self, position: u64) -> String {
        format!("{}:{position}", self.generation)
    }
    pub fn position(&self, cursor: &str) -> Result<u64> {
        let (generation, position) = cursor.rsplit_once(':').ok_or(BrowserError::StaleCursor)?;
        let position = position
            .parse::<u64>()
            .map_err(|_| BrowserError::StaleCursor)?;
        if generation != self.generation || position > self.next {
            return Err(BrowserError::StaleCursor);
        }
        Ok(position)
    }
    pub fn truncated_since(&self, position: u64) -> bool {
        self.evicted_through
            .is_some_and(|sequence| position <= sequence)
    }
    pub fn has_evicted(&self) -> bool {
        self.evicted_through.is_some()
    }
}
