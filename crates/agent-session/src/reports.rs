//! Command reports (`shell-tool.md` § Command reports): the commands the
//! session's `shell` calls left running, each with how often it reports and
//! the task that watches it. The task reports the command's progress every
//! interval while it runs, and its end once it ends; what each report says
//! is the node's to tell ([`SessionRuntime::report`]), and the session admits
//! it as input waiting for a boundary.
//!
//! [`SessionRuntime::report`]: crate::SessionRuntime::report

use tracing::Instrument as _;
use std::{
    rc::{Rc, Weak},
    time::Duration,
};

use demi_agent_store::CommandInterval;
use demi_shared_types::{CommandId, ReportEvent};
use tokio_util::task::AbortOnDropHandle;

use super::{SessionShared, input::Opens};

/// The commands the session's calls left running, in the order they were
/// left running.
#[derive(Default)]
pub(super) struct Watched {
    commands: Vec<WatchedCommand>,
}

struct WatchedCommand {
    command: CommandId,
    /// How often it reports while it runs; none for a resident command,
    /// which reports only its end.
    interval_ms: Option<u32>,
    /// The task that watches it, which goes with the entry.
    task: Option<AbortOnDropHandle<()>>,
}

impl Watched {
    /// Watches `command` from now on, every `interval_ms`, in place of a
    /// watch of it before.
    pub(super) fn add(&mut self, command: CommandId, interval_ms: Option<u32>) {
        self.remove(&command);
        self.commands.push(WatchedCommand {
            command,
            interval_ms,
            task: None,
        });
    }

    /// Keeps the task that watches `command`; a command no longer watched
    /// drops it, which stops it.
    pub(super) fn set_task(&mut self, command: &CommandId, task: AbortOnDropHandle<()>) {
        if let Some(watched) = self.find(command) {
            watched.task = Some(task);
        }
    }

    /// Changes how often `command` reports, from now on: its task goes, and
    /// the caller starts the next. False when it is not watched.
    pub(super) fn set_interval(&mut self, command: &CommandId, interval_ms: Option<u32>) -> bool {
        let Some(watched) = self.find(command) else {
            return false;
        };
        watched.interval_ms = interval_ms;
        watched.task = None;
        true
    }

    /// Stops watching `command`, and its task with it.
    pub(super) fn remove(&mut self, command: &CommandId) {
        self.commands.retain(|watched| &watched.command != command);
    }

    /// The interval of `command`, while it is watched.
    pub(super) fn get(&self, command: &CommandId) -> Option<Option<u32>> {
        self.commands
            .iter()
            .find(|watched| &watched.command == command)
            .map(|watched| watched.interval_ms)
    }

    /// Ends every watch and keeps the commands, as a disposed session's
    /// final checkpoint does.
    pub(super) fn stop_watching(&mut self) {
        for watched in &mut self.commands {
            watched.task = None;
        }
    }

    /// The commands watched, in the order they were left running.
    pub(super) fn commands(&self) -> Vec<CommandId> {
        self.commands
            .iter()
            .map(|watched| watched.command.clone())
            .collect()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Each command watched with its interval, as the checkpoint keeps it.
    pub(super) fn intervals(&self) -> Vec<CommandInterval> {
        self.commands
            .iter()
            .map(|watched| CommandInterval {
                command_id: watched.command.clone(),
                interval_ms: watched.interval_ms,
            })
            .collect()
    }

    fn find(&mut self, command: &CommandId) -> Option<&mut WatchedCommand> {
        self.commands
            .iter_mut()
            .find(|watched| &watched.command == command)
    }
}

/// Starts the task that watches `command`, which the session watches, and
/// gives it to the session. Its reports count from its start, which was
/// `ran` ago when its call watched it that long (`shell-tool.md` § Command
/// reports): the first comes at the next whole interval since then.
pub(super) fn start_watch(s: &Rc<SessionShared>, command: &CommandId, ran: Duration) {
    let Some(interval_ms) = s.read(|core| core.watched.get(command)) else {
        return;
    };
    let task = tokio::task::spawn_local(
        watch(Rc::downgrade(s), command.clone(), interval_ms, ran).in_current_span(),
    );
    s.update(|core| core.watched.set_task(command, AbortOnDropHandle::new(task)));
}

/// Reports `command`'s progress every `interval_ms` from its start, `ran`
/// ago, while it runs, and its end once it ends; then the session no longer
/// watches it. The task holds the session only while it hands it a report.
async fn watch(session: Weak<SessionShared>, command: CommandId, interval_ms: Option<u32>, mut ran: Duration) {
    let Some(s) = session.upgrade() else {
        return;
    };
    let runtime = s.runtime.clone();
    drop(s);
    let ended = runtime.command_ended(&command);
    tokio::pin!(ended);
    loop {
        let progress = async {
            match interval_ms {
                Some(interval) => {
                    let interval = u64::from(interval);
                    let since = u64::try_from(ran.as_millis()).unwrap_or(u64::MAX) % interval;
                    tokio::time::sleep(Duration::from_millis(interval - since)).await;
                }
                None => std::future::pending().await,
            }
        };
        let ended_now = tokio::select! {
            () = &mut ended => true,
            () = progress => false,
        };
        ran = Duration::ZERO;
        report(&session, &command).await;
        if ended_now {
            if let Some(s) = session.upgrade() {
                // This drops the task's own handle, which aborts nothing
                // more: the task ends here, before any further await.
                s.update(|core| core.watched.remove(&command));
            }
            return;
        }
    }
}

/// Asks the node for `command`'s report now, and admits it as input when
/// the node has one to tell.
async fn report(session: &Weak<SessionShared>, command: &CommandId) {
    let Some(s) = session.upgrade() else {
        return;
    };
    let Some(interval_ms) = s.read(|core| core.watched.get(command)) else {
        return;
    };
    let (model, limits) = s.read(|core| (core.model.clone(), core.request_limits()));
    let runtime = s.runtime.clone();
    drop(s);
    let Some(report) = runtime
        .report(command, interval_ms, &model, limits)
        .await
    else {
        return;
    };
    // The loss of a resident command opens no turn: nothing waits for it
    // (`shell-tool.md` § Lost commands).
    let opens = match (&report.event, interval_ms) {
        (ReportEvent::Lost { .. }, None) => Opens::Nothing,
        _ => Opens::Turn,
    };
    if let Some(s) = session.upgrade() {
        s.update(|core| core.admit_report(report, opens));
    }
}
