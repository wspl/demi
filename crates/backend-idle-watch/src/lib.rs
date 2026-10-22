//! The idle rule's mechanism: one idle window, and one watch per resource.
//!
//! The rule itself, what counts as activity and what a retirement does, belongs
//! to `crates/backend-user-shard/src/lifecycle/mod.rs`.
//!
//! A watch reads its resource's gates:
//!
//! - The resource is idle once its last demand ended.
//! - New demand, however brief, restarts a full window.
//! - Maintenance holds a gate without demand. It postpones a retirement without
//!   restarting the window, and the retirement follows its release.
//!
//! At the deadline the watch reserves the resource, reads it again under the
//! reservation and only then retires it, so work that won admission first is
//! never retired.
//!
//! The watch does not know what its resource is: its callers read, reserve and
//! retire it through an [`IdlePolicy`]. A watch never asks a Host or a service
//! whether it is busy: activity is what the resource's gates admitted.

use std::future::Future;
use std::time::Duration;

use demi_shared_gates::GateState;
use futures_util::future::LocalBoxFuture;
use tokio::time::Instant;
use tokio_util::task::TaskTracker;

/// What a watch reads of its resource: whether work holds it now, and when
/// its last demand ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    pub busy: bool,
    pub last_demand_end: Option<Instant>,
}

impl Activity {
    /// The activity `gate` shows: demand holds it, or held it last.
    pub fn of(gate: &GateState) -> Self {
        Self {
            busy: gate.demand > 0,
            last_demand_end: gate.last_demand_end,
        }
    }

    /// Both together: busy when either is, with the later demand end.
    pub fn and(self, other: Self) -> Self {
        Self {
            busy: self.busy || other.busy,
            last_demand_end: self.last_demand_end.max(other.last_demand_end),
        }
    }
}

/// A resource's retirement, which holds the resource's reservation until
/// it ends; dropped before it starts, it lets the reservation go.
pub type Retirement = LocalBoxFuture<'static, Result<(), String>>;

/// How a watch reads, reserves and retires its resource.
pub trait IdlePolicy {
    /// The resource's activity now.
    fn check(&self) -> impl Future<Output = Result<Activity, String>>;

    /// Reserves the resource for its retirement; none while work or
    /// maintenance holds it.
    fn reserve(&self) -> impl Future<Output = Result<Option<Retirement>, String>>;

    /// Resolves once something that may have held the resource let go, for
    /// a watch that found it held.
    fn changed(&self) -> impl Future<Output = ()>;
}

/// Watches `policy`'s resource until the watch retires it.
///
/// Once the resource has been idle for `window`, the watch reserves it, reads
/// it again and retires it. Activity is read every `poll`. A retirement that
/// failed, or a reading that did, is tried again a `poll` later, never in a
/// tight loop.
///
/// A line the watch logs names no resource: the caller runs the watch in a
/// span that holds the resource's ID (`conversation`, `host_id`).
///
/// The retirement is a task of `tasks`, so a watch that is stopped lets a
/// retirement already running finish, and the owner's close waits for it.
pub async fn watch(policy: impl IdlePolicy, window: Duration, poll: Duration, tasks: TaskTracker) {
    let mut idle_since: Option<Instant> = None;
    loop {
        let activity = match policy.check().await {
            Ok(activity) => activity,
            Err(error) => {
                tracing::warn!(event = "idle_watch.failed", step = "resource", %error, "an idle watch could not read its resource");
                tokio::time::sleep(poll).await;
                continue;
            }
        };
        let now = Instant::now();
        idle_since = idle_start(idle_since, activity, now);
        let Some(since) = idle_since else {
            tokio::time::sleep(poll).await;
            continue;
        };
        let deadline = since + window;
        if now < deadline {
            tokio::time::sleep_until(deadline.min(now + poll)).await;
            continue;
        }
        let retirement = match policy.reserve().await {
            Ok(Some(retirement)) => retirement,
            Ok(None) => {
                // Held by maintenance or a transition: the window stays
                // elapsed, and the retirement follows the release.
                tokio::select! {
                    () = policy.changed() => {}
                    () = tokio::time::sleep(poll) => {}
                }
                continue;
            }
            Err(error) => {
                tracing::warn!(event = "idle_watch.failed", step = "reserve", %error, "an idle watch could not reserve its resource");
                tokio::time::sleep(poll).await;
                continue;
            }
        };
        // Read again under the reservation: demand that came and went while
        // the reservation was taken restarts the window.
        let again = match policy.check().await {
            Ok(again) => again,
            Err(error) => {
                tracing::warn!(event = "idle_watch.failed", step = "read_reserved", %error, "an idle watch could not read its reserved resource");
                drop(retirement);
                tokio::time::sleep(poll).await;
                continue;
            }
        };
        if again.busy || again.last_demand_end.is_some_and(|end| end > since) {
            drop(retirement);
            idle_since = idle_start(idle_since, again, Instant::now());
            continue;
        }
        let retiring = tasks.spawn_local(retirement);
        match retiring.await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => {
                tracing::warn!(event = "idle_watch.failed", step = "retire", retry = true, %error, "a retirement failed and is tried again later");
                tokio::time::sleep(poll).await;
            }
            // The retirement panicked, and the watch ends with it.
            Err(_) => return,
        }
    }
}

/// When the resource became idle, given when it was idle since and what it
/// shows `now`: never while busy, and never before its last demand ended.
fn idle_start(since: Option<Instant>, activity: Activity, now: Instant) -> Option<Instant> {
    if activity.busy {
        return None;
    }
    let start = since.unwrap_or(now);
    Some(match activity.last_demand_end {
        Some(end) if end > start => end,
        _ => start,
    })
}
