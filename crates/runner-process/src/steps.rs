//! The steps that may hold up the runner's control thread, and the watch that
//! says when it was held up.
//!
//! ```text
//!   control thread: ... step("watch_teardown") ... drop ... beat every 100 ms
//!                         |  step.started          | step.ended
//!   watch thread:  sees no beat for 1 s --> control_loop.lagged, with the
//!                  last step that began
//! ```
//!
//! The control thread runs everything that ties the runner to its backend,
//! and a call that blocks it, such as a join on a thread that never ends,
//! stops the connection without an error. A step that may block writes
//! `step.started` before it and `step.ended` after it, so a start without an
//! end names the step that is stuck. The watch runs on a thread of its own,
//! which a stuck control thread does not stop, so it writes its line while
//! the stall lasts, and again when it ends with its length.

use std::{
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use tokio_util::task::AbortOnDropHandle;

/// A control thread that has not run for this long has lagged. A watch with
/// no threshold would write a line for every pause of the scheduler, and
/// none of them say that the thread is stuck.
pub const CONTROL_LAG: Duration = Duration::from_secs(1);

/// How often the control thread beats, and the watch looks.
const BEAT: Duration = Duration::from_millis(100);
const LOOK: Duration = Duration::from_millis(250);

static LAST: Mutex<&'static str> = Mutex::new("none");
static OPEN: AtomicUsize = AtomicUsize::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// A step in progress: `step.ended` is written when it is dropped.
#[must_use = "a step ends when it is dropped"]
pub struct Step {
    name: &'static str,
    id: String,
    began: Instant,
}

/// Begins the step `name` of `id`, such as a subscription or a thread.
pub fn step(name: &'static str, id: impl Into<String>) -> Step {
    let id = id.into();
    *LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = name;
    OPEN.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(event = "step.started", step = name, id = id.as_str(), "a step began");
    Step {
        name,
        id,
        began: Instant::now(),
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        OPEN.fetch_sub(1, Ordering::Relaxed);
        tracing::debug!(
            event = "step.ended",
            step = self.name,
            id = self.id.as_str(),
            ms = self.began.elapsed().as_millis() as u64,
            "a step ended"
        );
    }
}

/// The watch of the control thread; it ends when dropped.
pub struct ControlWatch {
    _beating: AbortOnDropHandle<()>,
    _beat: Arc<AtomicU64>,
}

/// Starts the watch of the thread it is called on, which runs a Tokio
/// runtime.
pub fn watch_control_loop() -> ControlWatch {
    let beat = Arc::new(AtomicU64::new(now_ms()));
    let beating = {
        let beat = beat.clone();
        AbortOnDropHandle::new(tokio::spawn(async move {
            let mut ticks = tokio::time::interval(BEAT);
            loop {
                ticks.tick().await;
                beat.store(now_ms(), Ordering::Relaxed);
            }
        }))
    };
    let watched = Arc::downgrade(&beat);
    // A thread that cannot start leaves the control thread unwatched, which
    // changes nothing it does.
    let _ = std::thread::Builder::new()
        .name("control-watch".into())
        .spawn(move || look(watched));
    ControlWatch {
        _beating: beating,
        _beat: beat,
    }
}

fn look(beat: Weak<AtomicU64>) {
    let mut lagged_since: Option<u64> = None;
    loop {
        std::thread::sleep(LOOK);
        let Some(beat) = beat.upgrade() else {
            return;
        };
        let last = beat.load(Ordering::Relaxed);
        let silent = now_ms().saturating_sub(last);
        let last_step = *LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let in_step = OPEN.load(Ordering::Relaxed) > 0;
        match lagged_since {
            None if silent >= CONTROL_LAG.as_millis() as u64 => {
                lagged_since = Some(last);
                tracing::warn!(
                    event = "control_loop.lagged",
                    ms = silent,
                    last_step,
                    in_step,
                    ended = false,
                    "the control thread has not run for a second"
                );
            }
            Some(since) if last > since => {
                lagged_since = None;
                tracing::warn!(
                    event = "control_loop.lagged",
                    ms = last.saturating_sub(since),
                    last_step,
                    in_step,
                    ended = true,
                    "the control thread ran again"
                );
            }
            _ => {}
        }
    }
}
