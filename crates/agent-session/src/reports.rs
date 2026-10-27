//! Command reports: the commands the session's `shell` calls left running, each
//! with how it wakes the node and the task that watches it.
//!
//! The task reports the command's end once it ends, and, for a command that is
//! not a service, once when it has printed nothing for the runtime's
//! silence limit, which says it still runs. What each report says is the node's
//! to tell ([`SessionRuntime::report`]), and the session admits it as input
//! waiting for a boundary.
//!
//! [`SessionRuntime::report`]: crate::SessionRuntime::report
//!
//! The model never asks to be woken: the work it waits for wakes it, as a
//! background command or a subagent wakes Claude Code's model.
//!
//! - Every end of a command whose call returned is reported, except one the
//!   model saw in a look or caused itself with `demi shell stop`. An end report
//!   carries the output and media a result would, because a model woken by an
//!   end without its output spent one more request on a look every time.
//! - A command still running after the silence limit with no output is the one
//!   thing besides an end that wakes the node, and only once, so a hung command
//!   reaches it and a slow one that prints nothing costs it one request. The
//!   limit counts from the command's last output, and again from the moment a
//!   command that waited for the user's answer goes on.
//! - A report names its command by number and the title of the call that
//!   started it, which the command's record keeps, so it reads the same
//!   whatever became of the call or the process that watched it.
//! - A command whose Host's connection is away sends no silence report, since
//!   the silence is the connection's and the node could do nothing with one.
//!
//! The loss of a service opens no turn: it joins the running turn at
//! its next boundary or waits for the next turn, so a conversation left idle
//! with its dev server running is not woken, and charged a request, by every
//! upgrade that replaces its runner. A lost command that is not a
//! service still wakes its node, which may be waiting for it.
//!
//! Reports not yet in the transcript and how each command wakes the node are
//! saved in the checkpoint, so both survive dispose and a restart.

use tracing::Instrument as _;
use std::{
    rc::{Rc, Weak},
    time::Duration,
};

use demi_agent_store::{CommandWatch, Watch};
use demi_shared_types::{CommandId, ReportEvent};
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;

use super::{SessionShared, input::Opens};
use crate::SessionRuntime;

/// The commands the session's calls left running, in the order they were
/// left running.
#[derive(Default)]
pub(super) struct Watched {
    commands: Vec<WatchedCommand>,
}

struct WatchedCommand {
    command: CommandId,
    watch: Watch,
    /// The task that watches it, which goes with the entry.
    task: Option<AbortOnDropHandle<()>>,
}

impl Watched {
    /// Watches `command` from now on as `watch` says, in place of a watch of
    /// it before.
    pub(super) fn add(&mut self, command: CommandId, watch: Watch) {
        self.remove(&command);
        self.commands.push(WatchedCommand {
            command,
            watch,
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

    /// Records that `command`'s silence was reported, so that only its end
    /// wakes the node from now on.
    fn silenced(&mut self, command: &CommandId) {
        if let Some(watched) = self.find(command) {
            watched.watch = Watch::Silenced;
        }
    }

    /// Stops watching `command`, and its task with it.
    pub(super) fn remove(&mut self, command: &CommandId) {
        self.commands.retain(|watched| &watched.command != command);
    }

    /// How `command` wakes the node, while it is watched.
    pub(super) fn get(&self, command: &CommandId) -> Option<Watch> {
        self.commands
            .iter()
            .find(|watched| &watched.command == command)
            .map(|watched| watched.watch)
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

    /// The commands watched that are not services: those whose end the node
    /// waits for.
    pub(super) fn awaited(&self) -> Vec<CommandId> {
        self.commands
            .iter()
            .filter(|watched| watched.watch != Watch::Service)
            .map(|watched| watched.command.clone())
            .collect()
    }

    /// Whether a command that is not a service is watched: one whose end the
    /// node waits for.
    pub(super) fn awaits(&self) -> bool {
        self.commands
            .iter()
            .any(|watched| watched.watch != Watch::Service)
    }

    /// Each command watched with how it wakes the node, as the checkpoint
    /// keeps it.
    pub(super) fn watches(&self) -> Vec<CommandWatch> {
        self.commands
            .iter()
            .map(|watched| CommandWatch {
                command_id: watched.command.clone(),
                watch: watched.watch,
            })
            .collect()
    }

    fn find(&mut self, command: &CommandId) -> Option<&mut WatchedCommand> {
        self.commands
            .iter_mut()
            .find(|watched| &watched.command == command)
    }
}

/// Starts the task that watches `command`, which the session watches, and gives
/// it to the session.
pub(super) fn start_watch(s: &Rc<SessionShared>, command: &CommandId) {
    let Some(watch) = s.read(|core| core.watched.get(command)) else {
        return;
    };
    let silence = s.config.silence;
    s.read(|core| {
        session_log!(
            core,
            info,
            event = "command_watch.started",
            node = %core.id,
            turn = core.running_turn().map(tracing::field::display),
            command = %command,
            wakes = if watch == Watch::Unreported { "end_and_silence" } else { "end" },
            silence_ms = u64::try_from(silence.as_millis()).unwrap_or(u64::MAX),
            "a command was watched for its end"
        );
    });
    let task = tokio::task::spawn_local(
        watch_command(Rc::downgrade(s), command.clone(), watch, silence).instrument(s.span.clone()),
    );
    s.update(|core| core.watched.set_task(command, AbortOnDropHandle::new(task)));
}

/// Reports `command`'s silence once, if it is not a service and has
/// not been reported, and its end once it ends; then the session no longer
/// watches it. The task holds the session only while it hands it a report.
async fn watch_command(session: Weak<SessionShared>, command: CommandId, watch: Watch, silence: Duration) {
    let Some(s) = session.upgrade() else {
        return;
    };
    let runtime = s.runtime.clone();
    drop(s);
    let ended = runtime.command_ended(&command);
    tokio::pin!(ended);
    let mut over = false;
    if watch == Watch::Unreported {
        // A silence the node has nothing to tell of, such as a command whose
        // Host is away, is looked at again after another silence.
        loop {
            let silent = next_silence(&*runtime, &command, silence);
            tokio::select! {
                () = &mut ended => {
                    over = true;
                    break;
                }
                () = silent => {}
            }
            if report(&session, &command).await {
                if let Some(s) = session.upgrade() {
                    s.update(|core| core.watched.silenced(&command));
                }
                break;
            }
            tokio::select! {
                () = &mut ended => {
                    over = true;
                    break;
                }
                () = tokio::time::sleep(silence) => {}
            }
        }
    }
    if !over {
        ended.await;
    }
    report(&session, &command).await;
    if let Some(s) = session.upgrade() {
        // This drops the task's own handle, which aborts nothing more: the
        // task ends here, before any further await.
        s.update(|core| core.watched.remove(&command));
    }
}

/// Waits until `command` has printed nothing for `silence`.
///
/// A command that waits for the user's answer is not silent, since nothing
/// about it changes until the user answers. Its silence counts again from the
/// moment it goes on.
async fn next_silence(runtime: &dyn SessionRuntime, command: &CommandId, silence: Duration) {
    // When the command last went on after waiting for the user.
    let mut resumed: Option<Instant> = None;
    loop {
        let quiet = runtime.quiet(command).unwrap_or_default();
        let quiet = resumed.map_or(quiet, |at| quiet.min(at.elapsed()));
        let left = silence.saturating_sub(quiet);
        // A job that has not started has not asked for anything.
        let started = tokio::select! {
            started = runtime.awaits_user(command) => started,
            () = tokio::time::sleep(left) => return,
        };
        let Some(mut awaits) = started else {
            tokio::time::sleep(left).await;
            return;
        };
        if *awaits.borrow_and_update() {
            // A job gone has ended, which the command's end tells.
            if awaits.wait_for(|awaits| !*awaits).await.is_err() {
                return std::future::pending().await;
            }
            resumed = Some(Instant::now());
            continue;
        }
        tokio::select! {
            () = tokio::time::sleep(left) => return,
            began = awaits.wait_for(|awaits| *awaits) => {
                if began.is_err() {
                    return std::future::pending().await;
                }
            }
        }
    }
}

/// Asks the node for `command`'s report now, and admits it as input when
/// the node has one to tell; whether it did.
async fn report(session: &Weak<SessionShared>, command: &CommandId) -> bool {
    let Some(s) = session.upgrade() else {
        return false;
    };
    let Some(watch) = s.read(|core| core.watched.get(command)) else {
        return false;
    };
    let (model, limits) = s.read(|core| (core.model.clone(), core.request_limits()));
    let runtime = s.runtime.clone();
    drop(s);
    let Some(report) = runtime.report(command, &model, limits).await else {
        return false;
    };
    // The loss of a service opens no turn: nothing waits for it.
    let opens = match (&report.event, watch) {
        (ReportEvent::Lost { .. }, Watch::Service) => Opens::Nothing,
        _ => Opens::Turn,
    };
    let Some(s) = session.upgrade() else {
        return false;
    };
    s.update(|core| core.admit_report(report, opens));
    true
}
