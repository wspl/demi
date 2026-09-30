//! Browser console and CDP streams share bounded retention and generation cursors.
use serde::Serialize;
use std::collections::VecDeque;

use super::{
    BrowserError, Result,
    protocol::{CdpEvent, LogEntry},
};

/// An entry whose position in its stream the buffer assigns.
pub(super) trait Entry: Serialize {
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

pub(super) struct Buffer<T> {
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
            generation: super::handles::fresh(prefix)?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::protocol::LogLevel;

    fn entry(text: &str) -> LogEntry {
        LogEntry {
            sequence: 0,
            level: LogLevel::Log,
            text: text.into(),
            url: None,
            timestamp: 0.0,
        }
    }

    #[test]
    fn browser_streams_keep_independent_cursors_and_report_count_and_byte_eviction() {
        let mut buffer = Buffer::new("test", 2, 160).unwrap();
        let start = buffer.cursor(0);
        for value in ["a", "b", "c"] {
            buffer.push(entry(value)).unwrap();
        }
        assert_eq!(
            buffer.entries().map(Entry::sequence).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(buffer.position(&start).unwrap(), 0);
        assert!(buffer.truncated_since(0));
        assert!(!buffer.truncated_since(1));
        assert_eq!(buffer.entries().count(), 2);
        let other = Buffer::<LogEntry>::new("test", 2, 160).unwrap();
        assert!(other.position(&start).is_err());
        assert!(buffer.position(&buffer.cursor(4)).is_err());
        buffer.push(entry(&"x".repeat(160))).unwrap();
        assert_eq!(buffer.entries().count(), 0);
        assert!(buffer.truncated_since(3));
        buffer.mark_gap().unwrap();
        assert_eq!(buffer.next(), 5);
        assert!(buffer.truncated_since(4));
        assert!(!buffer.truncated_since(5));
    }
}
