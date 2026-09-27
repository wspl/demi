//! Where a test holds runners' hellos (`runner.md` § Connection and
//! identity), for the scenarios that race a hello against its runner going
//! away and against the backend's shutdown (`Backend::hold_hellos`).

/// Where a hello waits while a test holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloStep {
    /// At the edge, as the runner's token is looked up, while the edge
    /// watches the runner's socket for its close.
    TokenLookup,
    /// In the shard of the device's owner, which took the socket and has not
    /// bound it to the device yet.
    Bind,
}
