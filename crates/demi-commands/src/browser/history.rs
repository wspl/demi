//! Browser console and CDP streams share bounded retention and generation cursors.
use super::{BrowserError, Result};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub(super) struct Buffer {
    generation: String,
    next: u64,
    bytes: usize,
    entry_limit: usize,
    byte_limit: usize,
    evicted_through: Option<u64>,
    entries: VecDeque<(Value, usize)>,
}

impl Buffer {
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

    pub fn push(&mut self, mut entry: Value) -> Result<()> {
        let object = entry.as_object_mut().ok_or_else(|| {
            BrowserError::InvalidResult("browser history entry must be an object".into())
        })?;
        if object.contains_key("sequence") {
            return Err(BrowserError::InvalidResult(
                "browser history owns entry sequences".into(),
            ));
        }
        object.insert("sequence".into(), json!(self.next));
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
                let sequence = entry["sequence"]
                    .as_u64()
                    .expect("buffer assigns each sequence");
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

    pub fn entries(&self) -> impl Iterator<Item = &Value> {
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

    #[test]
    fn browser_streams_keep_independent_cursors_and_report_count_and_byte_eviction() {
        let mut buffer = Buffer::new("test", 2, 128).unwrap();
        let start = buffer.cursor(0);
        for value in ["a", "b", "c"] {
            buffer.push(json!({"text":value})).unwrap();
        }
        assert_eq!(
            buffer
                .entries()
                .map(|entry| entry["sequence"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(buffer.position(&start).unwrap(), 0);
        assert!(buffer.truncated_since(0));
        assert!(!buffer.truncated_since(1));
        assert_eq!(buffer.entries().count(), 2);
        let other = Buffer::new("test", 2, 128).unwrap();
        assert!(other.position(&start).is_err());
        assert!(buffer.position(&buffer.cursor(4)).is_err());
        buffer.push(json!({"text":"x".repeat(128)})).unwrap();
        assert_eq!(buffer.entries().count(), 0);
        assert!(buffer.truncated_since(3));
        buffer.mark_gap().unwrap();
        assert_eq!(buffer.next(), 5);
        assert!(buffer.truncated_since(4));
        assert!(!buffer.truncated_since(5));
    }
}
