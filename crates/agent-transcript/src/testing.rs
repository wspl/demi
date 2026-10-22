//! Predictable identities, and the text the model receives for a resume,
//! which the compaction tests compare requests with (feature `testing`).

use std::cell::Cell;

use crate::IdSource;
pub use crate::replay::RESUME_TEXT;

/// Identities `<prefix>-1`, `<prefix>-2`, and on.
#[derive(Debug)]
pub struct SequentialIds {
    prefix: String,
    next: Cell<u64>,
}

impl SequentialIds {
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            next: Cell::new(1),
        }
    }
}

impl IdSource for SequentialIds {
    fn next_id(&self) -> String {
        let id = self.next.get();
        self.next.set(id + 1);
        format!("{}-{id}", self.prefix)
    }
}
