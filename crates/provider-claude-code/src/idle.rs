//! The idle CLI processes of a machine.
//!
//! A process is idle while no action uses it: a runtime's kept process between
//! the session's actions, and a prepared one before its request takes it
//! (`crates/provider-claude-code/src/prepared.rs`). A process waiting on its
//! tool calls belongs to its action and is never idle, however long its tools
//! run.
//!
//! An idle process holds about 100 MB of the machine's memory and saves its
//! next request about a second of starting. So every idle process takes a seat
//! among its machine's, and closes, SIGTERM first:
//!
//! - after five idle minutes, whatever pages show its conversation; the next
//!   request starts a new process, which resumes the session;
//! - when the machine has more idle processes than a quarter of its memory
//!   holds, the oldest first;
//! - a prepared one, when its entry's active account changed, since it runs in
//!   the previous account's directory.
//!
//! ```text
//! runtime (user's shard)        IdleProcesses (the machine's, any shard)
//!   action ends:  Kept ---seat---> [oldest ... newest]  <= memory / 4 / 128 MB
//!   prepare:  Prepared --seat--->      | five idle minutes, or beyond the
//!   next run takes it: seat freed      v limit: the holder closes it
//! ```

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::Shared;
use crate::live::LiveCli;
use crate::placement::Placement;

/// How long a process stays idle before it is closed.
///
/// In the replayed history of a Cloud used all day, 99% of the turns that took
/// a kept process came within 5.5 minutes of the last.
pub(crate) const IDLE_FOR: Duration = Duration::from_secs(5 * 60);

/// The memory an idle process is counted at: one measured 100 to 110 MB of its
/// own on Linux.
const PROCESS_BYTES: u64 = 128 * 1024 * 1024;

/// What an idle process is waiting as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Idle {
    /// Kept by a runtime after its action ended.
    Kept,
    /// Prepared ahead of the request it waits for.
    Prepared,
}

impl Idle {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Prepared => "prepared",
        }
    }
}

/// The idle CLI processes of one machine, of every runtime and user whose
/// processes start there, oldest first.
///
/// It lives with the machine on the backend, which tells it the machine's
/// memory as the machine's runner reports it.
#[derive(Default)]
pub struct IdleProcesses {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    seats: VecDeque<SeatRecord>,
    /// How many seats were taken, which numbers them.
    taken: u64,
    /// The machine's physical memory in bytes; none before its runner was
    /// reached, when no process has started there yet.
    memory: Option<u64>,
}

struct SeatRecord {
    id: u64,
    /// The provider entry whose runtime holds the process.
    entry: String,
    /// The account whose directory the process runs in; none for a provider
    /// built without one.
    account: Option<String>,
    idle: Idle,
    close: CancellationToken,
}

impl State {
    /// Tells the oldest seats beyond what the machine's memory holds to close.
    fn trim(&mut self) {
        let Some(memory) = self.memory else {
            return;
        };
        let most = usize::try_from(memory / 4 / PROCESS_BYTES).unwrap_or(usize::MAX);
        while self.seats.len() > most {
            let Some(oldest) = self.seats.pop_front() else {
                break;
            };
            oldest.close.cancel();
            tracing::info!(
                event = "claude_code_process.idle_closed",
                provider = oldest.entry,
                process = oldest.idle.as_str(),
                reason = "memory",
                limit = most,
                "an idle Claude Code process was closed for a newer one: the machine's memory holds no more"
            );
        }
    }
}

impl IdleProcesses {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // The state stays whole whatever panicked while it was held.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the machine's physical memory, `bytes`, as its runner reported it,
    /// and closes the oldest idle processes beyond what it holds.
    pub fn hold_memory(&self, bytes: u64) {
        let mut state = self.state();
        state.memory = Some(bytes);
        state.trim();
    }

    /// A seat for a process of `entry`'s runtime in the directory of
    /// `account`, idle from now; the oldest beyond the machine's memory is told
    /// to close.
    pub(crate) fn seat(self: &Arc<Self>, entry: &str, account: Option<&str>, idle: Idle) -> Seat {
        let close = CancellationToken::new();
        let mut state = self.state();
        let id = state.taken;
        state.taken += 1;
        state.seats.push_back(SeatRecord {
            id,
            entry: entry.to_owned(),
            account: account.map(str::to_owned),
            idle,
            close: close.clone(),
        });
        state.trim();
        Seat {
            id,
            set: Arc::downgrade(self),
            close,
            until: Instant::now() + IDLE_FOR,
        }
    }

    /// Closes every process prepared for the provider entry `entry` in another
    /// account's directory than `active`'s, the entry's active account, as
    /// when that changed.
    ///
    /// Every user's shard follows the change, so one prepared again in the
    /// active account's directory, by a shard that followed it first, stays.
    pub fn close_prepared_of(&self, entry: &str, active: Option<&str>) {
        self.close_prepared_where(|seat| seat.entry == entry && seat.account.as_deref() != active);
    }

    /// Closes every process prepared on the machine, as when the account
    /// changes they were prepared before were missed.
    pub fn close_prepared(&self) {
        self.close_prepared_where(|_| true);
    }

    fn close_prepared_where(&self, closes: impl Fn(&SeatRecord) -> bool) {
        self.state().seats.retain(|seat| {
            let closed = seat.idle == Idle::Prepared && closes(seat);
            if closed {
                seat.close.cancel();
            }
            !closed
        });
    }
}

/// An idle process's place among its machine's; leaving it, when the process
/// is used or closed, frees it.
pub(crate) struct Seat {
    id: u64,
    set: Weak<IdleProcesses>,
    close: CancellationToken,
    /// When the process has been idle too long.
    until: Instant,
}

/// Why a seat says its process closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closing {
    /// It was idle for five minutes.
    Unused,
    /// The machine needs its memory, or its account changed.
    Told,
}

impl Seat {
    /// Resolves when the process is to close, and says why.
    pub(crate) fn closing(&self) -> impl Future<Output = Closing> + 'static {
        let told = self.close.clone().cancelled_owned();
        let unused = tokio::time::sleep_until(self.until);
        async move {
            tokio::select! {
                () = told => Closing::Told,
                () = unused => Closing::Unused,
            }
        }
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        if let Some(set) = self.set.upgrade() {
            set.state().seats.retain(|seat| seat.id != self.id);
        }
    }
}

/// Logs that a process `idle` waiting as closed after five idle minutes.
pub(crate) fn log_unused(shared: &Shared, node: &str, idle: Idle) {
    tracing::info!(
        event = "claude_code_process.idle_closed",
        provider = %shared.id,
        node,
        process = idle.as_str(),
        reason = "unused",
        ms = u64::try_from(IDLE_FOR.as_millis()).unwrap_or(u64::MAX),
        "an idle Claude Code process was closed"
    );
}

/// A runtime's process kept from its last action, seated idle until the next
/// run takes it; meanwhile its seat may close it.
pub(crate) struct Kept {
    live: Rc<RefCell<Option<LiveCli>>>,
    _closer: AbortOnDropHandle<()>,
}

impl Kept {
    /// Seats `live`, which no action uses any more, among its machine's idle
    /// processes.
    pub(crate) fn seat(shared: &Arc<Shared>, placement: &dyn Placement, live: LiveCli) -> Self {
        let seat = placement.idle().seat(&shared.id, shared.auth.account().ok(), Idle::Kept);
        let live = Rc::new(RefCell::new(Some(live)));
        let closer = {
            let live = live.clone();
            let shared = shared.clone();
            let closing = seat.closing();
            tokio::task::spawn_local(async move {
                let closing = closing.await;
                drop(seat);
                let Some(live) = live.borrow_mut().take() else {
                    return;
                };
                if closing == Closing::Unused {
                    log_unused(&shared, &live.node(), Idle::Kept);
                }
                // A close of its own, which a run taking the process
                // meanwhile cannot cut short.
                tokio::task::spawn_local(live.close());
            })
        };
        Self {
            live,
            _closer: AbortOnDropHandle::new(closer),
        }
    }

    /// The process, which leaves its seat; none when its seat closed it.
    pub(crate) fn take(self) -> Option<LiveCli> {
        self.live.borrow_mut().take()
    }

    /// Whether the process, while it is still kept, could go on with
    /// `request`, whatever its input ([`LiveCli::continues`]).
    pub(crate) fn continues(&self, request: &demi_provider_common::InferenceRequest) -> bool {
        self.live
            .borrow()
            .as_ref()
            .is_some_and(|live| live.continues(request))
    }
}
