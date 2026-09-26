//! The action worker (`runtime.md` § Actions, § A turn): one task per
//! session runs one action at a time. An action holds the tree's admission
//! while it runs, records how it ended, saves its checkpoint, and then starts
//! the next waiting action; the session shows idle only once that save has
//! committed and nothing waits.

use std::rc::{Rc, Weak};

use demi_core::Block;
use tokio::sync::Notify;

use super::{
    ActionEnd, ErrorReport, SessionEvent, SessionShared, Settle, TurnError,
    cancel::{CancelReason, TurnCancel},
    compaction,
    core::{ActionKind, StartedAction, SwitchPoint},
    editing::{self, EditError},
    input::Take,
    persist, turn,
};
use crate::{
    store::BoundaryEdge,
    transcript::{resume_point, rewind},
};

/// How an action's body ended.
enum Outcome {
    Completed,
    Aborted,
    Failed(ErrorReport),
}

/// The worker: waits for an action to start, runs every action that starts
/// until the session settles, and ends once the session is closed. It holds
/// the session only while an action runs.
pub(crate) async fn run(session: Weak<SessionShared>, work: Rc<Notify>) {
    loop {
        work.notified().await;
        let Some(s) = session.upgrade() else {
            return;
        };
        while let Some(started) = s.update(|core| core.take_started()) {
            run_action(&s, started).await;
        }
        if s.read(|core| core.settle()) == Settle::Closed {
            return;
        }
    }
}

async fn run_action(s: &Rc<SessionShared>, started: StartedAction) {
    let StartedAction {
        kind,
        turn,
        cancel,
        reply,
    } = started;
    let lease = cancel.guard(s.runtime.enter_action()).await;
    let body = match &lease {
        Ok(_) => execute(s, &kind, &cancel).await,
        Err(_) => Err(TurnError::Cancelled),
    };
    // A stop that came in while the body finished its last step still stops
    // it: no action completes after its abort.
    let body = body.and_then(|()| cancel.check());
    if matches!(body, Err(TurnError::Cancelled)) && cancel.reason() == Some(CancelReason::Shutdown)
    {
        // Dispose writes the final checkpoint: it says the turn was running,
        // or it holds the message again when its turn had not begun.
        let interrupted = s.update(|core| core.shut_down(kind, &turn));
        cancel.acknowledge(false);
        if let Some(reply) = reply {
            let end = if interrupted {
                ActionEnd::Aborted
            } else {
                ActionEnd::Detached
            };
            // A caller that dropped its handle wants no answer.
            let _ = reply.send(Ok(end));
        }
        return;
    }
    if s.read(|core| core.preparing_edit()) {
        // A rejected edit changed nothing: it leaves no stop marker and no
        // failure record, and answers through its acceptance.
        let error = match body {
            Err(TurnError::Failed(report)) => EditError::Failed(report.message),
            Err(TurnError::Cancelled) | Ok(()) => EditError::Stopped,
        };
        s.update(|core| core.reject_edit(error));
        cancel.acknowledge(s.read(|core| core.can_abort_again()));
        s.update(|core| core.end_action());
        if let Some(reply) = reply {
            // A caller that dropped its handle wants no answer.
            let _ = reply.send(Ok(ActionEnd::Dropped));
        }
        drop(lease);
        s.update(|core| core.start_next());
        return;
    }
    let outcome = conclude(s, &cancel, body);
    cancel.acknowledge(s.read(|core| core.can_abort_again()));
    s.update(|core| core.end_action());
    // The closing save records the session idle, so a checkpoint that says
    // an action was running is one the process died in; clients see the
    // phase go idle only once it has committed, when the next action starts
    // or the session settles.
    let outcome = match (outcome, persist::flush(s).await) {
        (outcome, Ok(())) => outcome,
        (Outcome::Completed, Err(error)) => {
            let report = ErrorReport::from(error);
            s.emit(SessionEvent::Error {
                report: report.clone(),
            });
            Outcome::Failed(report)
        }
        (outcome, Err(error)) => {
            s.emit(SessionEvent::Error {
                report: error.into(),
            });
            outcome
        }
    };
    let result = match outcome {
        Outcome::Completed => Ok(ActionEnd::Completed),
        Outcome::Aborted => Ok(ActionEnd::Aborted),
        Outcome::Failed(report) => {
            s.emit(SessionEvent::ActionFailed {
                report: report.clone(),
            });
            Err(Box::new(report))
        }
    };
    if let Some(reply) = reply {
        // A caller that dropped its handle wants no answer.
        let _ = reply.send(result);
    }
    drop(lease);
    s.update(|core| core.start_next());
}

async fn execute(
    s: &Rc<SessionShared>,
    kind: &ActionKind,
    cancel: &TurnCancel,
) -> Result<(), TurnError> {
    match kind {
        ActionKind::Send { content } => {
            // The version current when the turn started, before its hooks
            // and commands run.
            let revision = s.read(|core| core.commands.revision());
            turn::apply_switch(s, SwitchPoint::ActionStart, cancel).await?;
            let preamble = cancel.guard(s.runtime.preamble()).await?;
            s.update(|core| {
                let turn = core.turn();
                core.push_user(turn, content.clone(), preamble, revision);
            });
            compaction::preflight(s, cancel).await?;
            turn::run(s, cancel).await
        }
        ActionKind::Continue => {
            turn::apply_switch(s, SwitchPoint::ActionStart, cancel).await?;
            let Some(agent_message) = s.update(|core| core.open_continuation()) else {
                // What woke it was taken by an earlier action.
                return Ok(());
            };
            if agent_message {
                persist::flush(s).await?;
            }
            compaction::preflight(s, cancel).await?;
            turn::run(s, cancel).await
        }
        ActionKind::Retry => retry(s, cancel).await,
        ActionKind::Resume => resume(s, cancel).await,
        ActionKind::Edit => editing::run(s, cancel).await,
        ActionKind::Compact => {
            compaction::compacting(s, compaction::run_pass(s, cancel)).await?;
            // Agent messages that were waiting reach the model; steers that
            // arrived during the pass wait in the transcript for the next
            // turn.
            let agent_input = s.read(|core| core.inputs.has_agent_input());
            if s.update(|core| core.write_inputs(Take::Everything)) {
                persist::flush(s).await?;
            }
            if agent_input {
                return turn::run(s, cancel).await;
            }
            Ok(())
        }
    }
}

/// Discards the last input turn and runs it again from its input: the block
/// that opened it stays, with the turn's steers and every agent message after
/// it, and command state returns to the version recorded before it
/// (`failures-and-recovery.md` § Recovery is one mechanism).
async fn retry(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    let rewind = s
        .read(|core| rewind(core.transcript.blocks()))
        .ok_or_else(|| TurnError::refused("There is no input turn to retry"))?;
    let input = &rewind.retained[rewind.input];
    let edge = match input {
        Block::AgentMessage(_) => BoundaryEdge::AfterBlock,
        _ => BoundaryEdge::BeforeUser,
    };
    let revision = s
        .read(|core| core.commands.boundary(input.id(), edge))
        .ok_or_else(|| {
            TurnError::refused(format!(
                "No command-state boundary for block {}",
                input.id()
            ))
        })?;
    let turn = rewind.turn.clone();
    persist::commit_rewrite(s, rewind.retained, revision).await?;
    s.update(|core| core.take_over_turn(turn));
    cancel.check()?;
    turn::apply_switch(s, SwitchPoint::ActionStart, cancel).await?;
    compaction::preflight(s, cancel).await?;
    turn::run(s, cancel).await
}

/// Finishes an unfinished turn: unwinds it to its resume point, marks the
/// stop it continues as resumed, appends a `resume` block and infers again.
/// A turn that left nothing but leftovers runs again as a retry.
async fn resume(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    let point = s.read(|core| resume_point(core.transcript.blocks()));
    if point.full_rerun {
        return retry(s, cancel).await;
    }
    // The unwind comes before a pending switch lands: a switch that compacts
    // would move the cut.
    turn::restore_command_state(s, point.cut).await?;
    // A stop that came during the save is recorded after the published
    // rewrite, and no `resume` block without a turn is left.
    cancel.check()?;
    s.update(|core| core.mark_abort_resumed());
    turn::apply_switch(s, SwitchPoint::ActionStart, cancel).await?;
    s.update(|core| core.push_resume());
    compaction::preflight(s, cancel).await?;
    turn::run(s, cancel).await
}

/// Records how the body ended, unless dispose stopped it: a stop leaves its
/// marker in the transcript, a failure is reported.
fn conclude(s: &SessionShared, cancel: &TurnCancel, body: Result<(), TurnError>) -> Outcome {
    match body {
        Ok(()) => Outcome::Completed,
        Err(TurnError::Cancelled) => {
            let reason = cancel
                .reason()
                .expect("a stopped action's token was cancelled with its reason");
            s.update(|core| core.record_stop(reason));
            Outcome::Aborted
        }
        Err(TurnError::Failed(report)) => {
            s.update(|core| core.fail(&report));
            Outcome::Failed(*report)
        }
    }
}
