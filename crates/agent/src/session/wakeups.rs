//! The wakeup driver (`runtime.md` § Yield wakeups): one task per session
//! that sleeps until the earliest wakeup is due, in wall-clock time, and
//! fires it. A change of the scheduled wakeups makes it plan again.

use std::{
    rc::{Rc, Weak},
    sync::Arc,
    time::Duration,
};

use demi_core::Clock;
use tokio::sync::Notify;

use super::SessionShared;

pub(super) async fn drive(session: Weak<SessionShared>, replan: Rc<Notify>, clock: Arc<dyn Clock>) {
    loop {
        let Some(due) = session
            .upgrade()
            .and_then(|s| s.read(|core| core.next_wakeup()))
        else {
            replan.notified().await;
            continue;
        };
        let wait = due.as_millisecond() - clock.now().as_millisecond();
        let wait = Duration::from_millis(u64::try_from(wait).unwrap_or(0));
        tokio::select! {
            () = tokio::time::sleep(wait) => {
                let Some(s) = session.upgrade() else {
                    return;
                };
                s.update(|core| core.fire_due_wakeups());
            }
            () = replan.notified() => {}
        }
    }
}
