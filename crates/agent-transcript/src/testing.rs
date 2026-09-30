//! Test support (feature `testing`): predictable identities, and the texts
//! the model receives for a resume, a fired wakeup and an agent message,
//! which a session's tests compare its requests with.

use std::cell::Cell;

use crate::IdSource;
pub use crate::replay::{RESUME_TEXT, WAKEUP_TEXT, agent_message_envelope};

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
