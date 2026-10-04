//! The action worker (`runtime.md` § Actions, § A turn): one task per
//! session runs one action at a time. An action holds the tree's admission
//! while it runs, records how it ended, saves its checkpoint, and then starts
//! the next waiting action; the session shows idle only once that save has
//! committed and nothing waits.

use std::rc::{Rc, Weak};

use demi_agent_transcript::{resume_point, rewind, unwind};
use demi_shared_types::TurnId;
use tokio::sync::Notify;

use super::{
    ActionEnd, ErrorReport, SessionEvent, SessionShared, Settle, TurnError,
    cancel::{CancelReason, TurnCancel},
    compaction,
    core::{ActionKind, StartedAction},
    editing::{self, EditError},
    input::Take,
    persist, turn,
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
    // Clients see the usage the action left before the phase goes idle. A
    // stopped action's token is cancelled, so the estimate reads with one
    // nothing stops.
    compaction::report_context_usage(s, &TurnCancel::new()).await;
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
        // The input is written before a recorded switch lands, so a switch
        // that fails or is stopped ends a turn that holds it
        // (`failures-and-recovery.md` § The unfinished turn).
        ActionKind::Send { content } => {
            let preamble = cancel.guard(s.runtime.preamble()).await?;
            s.update(|core| {
                let turn = core.turn();
                core.push_user(turn, content.clone(), preamble);
            });
            turn::apply_switch(s, cancel).await?;
            compaction::preflight(s, cancel).await?;
            turn::run(s, cancel).await
        }
        ActionKind::Continue => {
            let Some(agent_message) = s.update(|core| core.open_continuation()) else {
                // What woke it was taken by an earlier action.
                return Ok(());
            };
            if agent_message {
                persist::flush(s).await?;
            }
            turn::apply_switch(s, cancel).await?;
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
/// it (`failures-and-recovery.md` § Recovery is one mechanism).
async fn retry(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    let rewind = s
        .read(|core| rewind(core.transcript.blocks()))
        .ok_or_else(|| TurnError::refused("There is no input turn to retry"))?;
    persist::commit_rewrite(s, rewind.retained).await?;
    rerun(s, cancel, rewind.turn).await
}

/// Runs `turn` again from its input, which is the history's last turn.
async fn rerun(s: &Rc<SessionShared>, cancel: &TurnCancel, turn: TurnId) -> Result<(), TurnError> {
    s.update(|core| core.take_over_turn(turn));
    cancel.check()?;
    turn::apply_switch(s, cancel).await?;
    compaction::preflight(s, cancel).await?;
    turn::run(s, cancel).await
}

/// Finishes an unfinished turn: unwinds it to its resume point, keeping the
/// compactions after it, marks the stop it continues as resumed, appends a
/// `resume` block and infers again. A turn that left nothing but leftovers
/// runs again from its input, as a retry does.
async fn resume(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    let point = s.read(|core| resume_point(core.transcript.blocks()));
    // The unwind comes before a pending switch lands: a switch that compacts
    // would move the cut.
    let kept = s.read(|core| unwind(core.transcript.blocks(), point.cut));
    persist::commit_rewrite(s, kept).await?;
    if let Some(turn) = point.rerun {
        return rerun(s, cancel, turn).await;
    }
    // A stop that came during the save is recorded after the published
    // rewrite, and no `resume` block without a turn is left.
    cancel.check()?;
    s.update(|core| core.mark_abort_resumed());
    turn::apply_switch(s, cancel).await?;
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
