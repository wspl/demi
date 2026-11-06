//! The times the server's pages' connections run with.
//!
//! The configuration sets them and tests shorten them
//! (`crates/backend/src/config/mod.rs`).

use std::time::Duration;

/// How the backend times a page's connection. Tests shorten the times.
#[derive(Debug, Clone, Copy)]
pub struct PageTuning {
    /// A connection that has written nothing for this long writes a
    /// heartbeat, so that its page can tell it from a dead one.
    pub heartbeat: Duration,
    /// How long a connection's close frame waits for a page that does not
    /// read; a page that has not taken it by then loses the connection
    /// without it (`crates/backend/src/backend.rs`).
    pub close_wait: Duration,
}

impl Default for PageTuning {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(30),
            close_wait: Duration::from_secs(1),
        }
    }
}
