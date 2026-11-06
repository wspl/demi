//! A request's connection, as its handler sees it: the address it came from,
//! and a control that closes it at once or once no byte moved on it for a
//! while.
//!
//! The server's listener makes one control for each HTTP/1 connection, and
//! its connection's IO notes each byte that moves. A program's process makes
//! one for each HTTP/2 stream, whose bodies note their bytes, since closing
//! the connection there would close every other request on it.

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use demi_runner_protocol::wire::WriteProgress;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

/// A connection as a request sees it: its control, and the address it came
/// from.
#[derive(Clone)]
pub struct Peer {
    pub control: ConnectionControl,
    /// None for a local socket, such as the runner socket.
    pub address: Option<IpAddr>,
}

/// Closes a connection from the request it serves: at once, or once no
/// byte moved on it for a while. Cloning it is cheap.
#[derive(Clone)]
pub struct ConnectionControl(Arc<ControlState>);

struct ControlState {
    /// Cancelled, the connection's IO fails.
    closed: CancellationToken,
    opened: Instant,
    /// When bytes last moved, in milliseconds after `opened`.
    moved: AtomicU64,
    /// Whether the connection's writes move, which the runner's socket
    /// asks for ([`ConnectionControl::write_progress`]).
    writes: Arc<WriteProgress>,
}

impl ControlState {
    fn touch(&self) {
        let elapsed = u64::try_from(self.opened.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.moved.store(elapsed, Ordering::Relaxed);
    }

    /// When bytes last moved.
    fn last_moved(&self) -> Instant {
        self.opened + Duration::from_millis(self.moved.load(Ordering::Relaxed))
    }
}

impl ConnectionControl {
    /// The control of a connection that opens now, which closing `closed`
    /// closes.
    pub fn new(closed: CancellationToken) -> Self {
        Self(Arc::new(ControlState {
            closed,
            opened: Instant::now(),
            moved: AtomicU64::new(0),
            writes: Arc::default(),
        }))
    }

    /// Notes that bytes moved on the connection now.
    pub fn moved(&self) {
        self.0.touch();
    }

    /// Where the connection's IO notes whether its writes move.
    pub fn writes(&self) -> &WriteProgress {
        &self.0.writes
    }

    /// Whether the connection's writes move: the runner's socket ends when
    /// they stop for too long (`crates/runner-protocol/src/wire/mod.rs`).
    pub fn write_progress(&self) -> Arc<WriteProgress> {
        self.0.writes.clone()
    }

    /// Closes the connection now.
    pub fn close(&self) {
        self.0.closed.cancel();
    }

    /// A token that closes the connection when cancelled, for what outlives
    /// the request, such as a pipe whose device reads it on this connection.
    pub fn closer(&self) -> CancellationToken {
        self.0.closed.clone()
    }

    /// Closes the connection once no byte moved on it for `limit`, counted
    /// from now, or once `ended` is cancelled, until the answer is dropped.
    pub fn watch(&self, limit: Duration, ended: CancellationToken) -> ConnectionWatch {
        self.0.touch();
        let state = self.0.clone();
        let watching = tokio::spawn(async move {
            loop {
                let due = state.last_moved() + limit;
                tokio::select! {
                    () = ended.cancelled() => break,
                    () = state.closed.cancelled() => return,
                    () = tokio::time::sleep_until(due) => {
                        // Bytes moved while this slept: the deadline moved.
                        if state.last_moved() + limit <= Instant::now() {
                            break;
                        }
                    }
                }
            }
            state.closed.cancel();
        });
        ConnectionWatch(AbortOnDropHandle::new(watching))
    }
}

/// A connection's watch; dropping it stops watching.
pub struct ConnectionWatch(
    #[expect(dead_code, reason = "held for its drop")] AbortOnDropHandle<()>,
);

