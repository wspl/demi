//! When Demi reclaims what a conversation uses on a Host
//! (`resource-lifecycle.md`): one idle window, and one watch per resource.
//! A watch reads its resource's gates: the resource is idle once its last
//! demand ended, and new demand, however brief, restarts a full window. At
//! the deadline the watch reserves the resource, reads it again under the
//! reservation and only then retires it, so work that won admission first
//! is never retired. Maintenance holds a gate without demand: it postpones
//! a retirement without restarting the window, and the retirement follows
//! its release. The Cloud stops at the device level (`managed`); a paired
//! device hears the conversation release ([`conversations`]).

pub(crate) mod conversations;

use std::any::Any;
use std::future::Future;
use std::time::Duration;

use demi_gates::GateState;
use futures_util::future::LocalBoxFuture;
use tokio::time::Instant;
use tokio_util::task::TaskTracker;

/// What a watch reads of its resource: whether work holds it now, and when
/// its last demand ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Activity {
    pub(crate) busy: bool,
    pub(crate) last_demand_end: Option<Instant>,
}

impl Activity {
    /// The activity `gate` shows: demand holds it, or held it last.
    pub(crate) fn of(gate: &GateState) -> Self {
        Self {
            busy: gate.demand > 0,
            last_demand_end: gate.last_demand_end,
        }
    }

    /// Both together: busy when either is, with the later demand end.
    pub(crate) fn and(self, other: Self) -> Self {
        Self {
            busy: self.busy || other.busy,
            last_demand_end: self.last_demand_end.max(other.last_demand_end),
        }
    }
}

/// A resource reserved for its retirement: what holds it, released when
/// the retirement ends, and the retirement itself.
pub(crate) struct Retirement {
    pub(crate) held: Box<dyn Any>,
    pub(crate) run: LocalBoxFuture<'static, Result<(), String>>,
}

/// How a watch reads, reserves and retires its resource.
pub(crate) trait IdlePolicy {
    /// The resource's activity now.
    fn check(&self) -> impl Future<Output = Result<Activity, String>>;

    /// Reserves the resource for its retirement; none while work or
    /// maintenance holds it.
    fn reserve(&self) -> impl Future<Output = Result<Option<Retirement>, String>>;

    /// Resolves once something that may have held the resource let go, for
    /// a watch that found it held.
    fn changed(&self) -> impl Future<Output = ()>;
}

/// Watches `policy`'s resource until it retired it: idle for `window`, it
/// is reserved, read again and retired. Activity is read every `poll`; a
/// retirement that failed, or a reading that did, is tried again a `poll`
/// later, never in a tight loop. The retirement is a task of `tasks`, so a
/// watch that is stopped lets a retirement already running finish, and the
/// owner's close waits for it.
pub(crate) async fn watch(policy: impl IdlePolicy, window: Duration, poll: Duration, tasks: TaskTracker) {
    let mut idle_since: Option<Instant> = None;
    loop {
        let activity = match policy.check().await {
            Ok(activity) => activity,
            Err(error) => {
                tracing::warn!("an idle watch could not read its resource: {error}");
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
                tracing::warn!("an idle watch could not reserve its resource: {error}");
                tokio::time::sleep(poll).await;
                continue;
            }
        };
        // Read again under the reservation: demand that came and went while
        // the reservation was taken restarts the window.
        let again = match policy.check().await {
            Ok(again) => again,
            Err(error) => {
                tracing::warn!("an idle watch could not read its reserved resource: {error}");
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
        let Retirement { held, run } = retirement;
        let retiring = tasks.spawn_local(async move {
            let _held = held;
            run.await
        });
        match retiring.await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => {
                tracing::warn!("a retirement failed and is tried again later: {error}");
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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use demi_gates::{ActivityGate, Purpose};
    use tokio::sync::oneshot;
    use tokio_util::task::AbortOnDropHandle;

    use super::*;

    const WINDOW: Duration = Duration::from_secs(3600);
    const POLL: Duration = Duration::from_secs(30);

    /// A resource that is one gate, whose retirement runs `retire`.
    struct Gated {
        gate: ActivityGate,
        retire: Box<dyn Fn() -> LocalBoxFuture<'static, Result<(), String>>>,
        /// Waited for before each reservation, when set.
        before_reserve: RefCell<Option<oneshot::Receiver<()>>>,
    }

    impl IdlePolicy for Rc<Gated> {
        async fn check(&self) -> Result<Activity, String> {
            Ok(Activity::of(&self.gate.state()))
        }

        async fn reserve(&self) -> Result<Option<Retirement>, String> {
            let waiting = self.before_reserve.borrow_mut().take();
            if let Some(waiting) = waiting {
                let _ = waiting.await;
            }
            Ok(self.gate.try_reserve().map(|reservation| Retirement {
                held: Box::new(reservation),
                run: (self.retire)(),
            }))
        }

        async fn changed(&self) {
            let _ = self.gate.subscribe().changed().await;
        }
    }

    /// A resource whose retirements are counted and stamped.
    fn counted() -> (Rc<Gated>, Rc<RefCell<Vec<Instant>>>) {
        let retired = Rc::new(RefCell::new(Vec::new()));
        let log = retired.clone();
        let gated = Rc::new(Gated {
            gate: ActivityGate::new(),
            retire: Box::new(move || {
                let log = log.clone();
                Box::pin(async move {
                    log.borrow_mut().push(Instant::now());
                    Ok(())
                })
            }),
            before_reserve: RefCell::new(None),
        });
        (gated, retired)
    }

    fn start(gated: &Rc<Gated>, tasks: &TaskTracker) -> AbortOnDropHandle<()> {
        AbortOnDropHandle::new(tasks.spawn_local(watch(gated.clone(), WINDOW, POLL, tasks.clone())))
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn maintenance_postpones_a_retirement_without_restarting_its_window_which_follows_the_release() {
        let (gated, retired) = counted();
        let tasks = TaskTracker::new();
        let started = Instant::now();
        let maintenance = gated.gate.try_enter(Purpose::Maintenance).unwrap();
        let _watch = start(&gated, &tasks);
        tokio::time::sleep(WINDOW * 2).await;
        assert!(retired.borrow().is_empty());
        let released = Instant::now();
        drop(maintenance);
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(*retired.borrow(), [released]);
        assert!(released - started >= WINDOW);
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn new_demand_restarts_a_full_window_and_nothing_is_retired_while_it_holds() {
        let (gated, retired) = counted();
        let tasks = TaskTracker::new();
        let _watch = start(&gated, &tasks);
        tokio::time::sleep(WINDOW / 2).await;
        let lease = gated.gate.enter(Purpose::Demand).await;
        tokio::time::sleep(WINDOW * 2).await;
        assert!(retired.borrow().is_empty());
        let ended = Instant::now();
        drop(lease);
        tokio::time::sleep(WINDOW - Duration::from_secs(1)).await;
        assert!(retired.borrow().is_empty());
        tokio::time::sleep(POLL + Duration::from_secs(1)).await;
        let retired = retired.borrow();
        assert_eq!(retired.len(), 1);
        // A full window after the demand ended, read at the watch's poll.
        assert!(retired[0] - ended >= WINDOW && retired[0] - ended <= WINDOW + POLL, "{:?}", retired[0] - ended);
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_stopped_watch_lets_its_retirement_finish_and_other_watches_go_on() {
        let tasks = TaskTracker::new();
        let (finish, finished) = oneshot::channel::<()>();
        let finished = Rc::new(RefCell::new(Some(finished)));
        let began = Rc::new(Cell::new(false));
        let done = Rc::new(Cell::new(false));
        let slow = Rc::new(Gated {
            gate: ActivityGate::new(),
            retire: Box::new({
                let (began, done, finished) = (began.clone(), done.clone(), finished.clone());
                move || {
                    let (began, done) = (began.clone(), done.clone());
                    let finished = finished.borrow_mut().take().expect("one retirement");
                    Box::pin(async move {
                        began.set(true);
                        let _ = finished.await;
                        done.set(true);
                        Ok(())
                    })
                }
            }),
            before_reserve: RefCell::new(None),
        });
        let (other, other_retired) = counted();
        let slow_watch = start(&slow, &tasks);
        let _other_watch = start(&other, &tasks);
        tokio::time::sleep(WINDOW + Duration::from_secs(1)).await;
        assert!(began.get());
        drop(slow_watch);
        assert_eq!(other_retired.borrow().len(), 1);
        tasks.close();
        let mut waiting = std::pin::pin!(tasks.wait());
        assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        finish.send(()).unwrap();
        waiting.await;
        assert!(done.get());
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_failed_retirement_releases_its_reservation_and_is_tried_again_a_poll_later() {
        let attempts = Rc::new(RefCell::new(Vec::new()));
        let log = attempts.clone();
        let failing = Rc::new(Gated {
            gate: ActivityGate::new(),
            retire: Box::new(move || {
                let log = log.clone();
                Box::pin(async move {
                    log.borrow_mut().push(Instant::now());
                    Err("release failed".to_owned())
                })
            }),
            before_reserve: RefCell::new(None),
        });
        let tasks = TaskTracker::new();
        let _watch = start(&failing, &tasks);
        tokio::time::sleep(WINDOW + Duration::from_secs(1)).await;
        assert_eq!(attempts.borrow().len(), 1);
        // The reservation went with the failure.
        assert!(!failing.gate.state().reserved);
        tokio::time::sleep(POLL).await;
        let attempts = attempts.borrow();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[1] - attempts[0], POLL);
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn brief_demand_while_the_reservation_is_taken_starts_a_fresh_window() {
        let (gated, retired) = counted();
        let (proceed, waiting) = oneshot::channel();
        gated.before_reserve.replace(Some(waiting));
        let tasks = TaskTracker::new();
        let _watch = start(&gated, &tasks);
        tokio::time::sleep(WINDOW + Duration::from_secs(1)).await;
        // The watch waits in its reservation; demand comes and goes.
        let lease = gated.gate.enter(Purpose::Demand).await;
        let ended = Instant::now();
        drop(lease);
        proceed.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(retired.borrow().is_empty());
        tokio::time::sleep(WINDOW).await;
        let retired = retired.borrow();
        assert_eq!(retired.len(), 1);
        assert!(retired[0] - ended >= WINDOW, "{:?}", retired[0] - ended);
    }
}
