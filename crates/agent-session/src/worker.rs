//! The action worker: one task per session runs one action at a time.
//!
//! An action holds the tree's admission while it runs, records how it ended,
//! saves its checkpoint, and then starts the next waiting action. The session
//! shows idle only once that save has committed and nothing waits.

use std::rc::{Rc, Weak};

use demi_agent_transcript::{opens_input_turn, resume_point, resume_point_within, rewind, unwind};
use demi_provider_common::Ahead;
use demi_shared_types::TurnId;
use tokio::sync::Notify;

use demi_conversation_socket_protocol::AgentCode;

use super::{
    ActionReply,
    ActionEnd, ErrorReport, SessionEvent, SessionShared, Settle, TurnError, ahead,
    cancel::{CancelReason, TurnCancel},
    compaction,
    core::{ActionKind, SessionCore, StartedAction},
    editing::{self, EditError},
    input::Take,
    persist, turn, window,
};

/// Delivers the events a turn's end held when the action ends, on every
/// path out of it.
struct ReleaseEvents<'a>(&'a SessionShared);

impl Drop for ReleaseEvents<'_> {
    fn drop(&mut self) {
        self.0.release_events();
    }
}

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
        loop {
            while let Some(started) = s.update(|core| core.take_started()) {
                run_action(&s, started).await;
            }
            // Idle now: what was asked to ready ahead is readied, unless an
            // action starts first.
            let Some(ahead) = s.update(SessionCore::take_ahead) else {
                break;
            };
            ahead::prepare(&s, ahead).await;
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
    // What the turn's end held reaches clients with the action's end, the
    // phase it leaves included, however the action ends.
    let _end = ReleaseEvents(s);
    let lease = cancel.guard(s.runtime.enter_action()).await;
    let body = match &lease {
        Ok(_) => execute(s, &kind, &cancel).await,
        Err(_) => Err(TurnError::Cancelled),
    };
    if matches!(kind, ActionKind::Switch { .. }) {
        end_switch(s, &cancel, body, reply).await;
        drop(lease);
        s.update(|core| core.start_next());
        return;
    }
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
            Err(TurnError::Failed(report)) => EditError::Failed(report),
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
    if matches!(outcome, Outcome::Aborted) {
        // A stop is answered once its record reached clients, and the
        // answer may go before the closing save ends.
        s.release_events();
    }
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
    s.update(|core| {
        // Nothing of the action runs any more; its requests have put the
        // runtime back in its slot.
        if let Some(runtime) = core.provider.held_mut() {
            runtime.idle();
        }
        // The action may have changed the history, and the token of a kept
        // process may be about to expire.
        core.ask_ahead(Ahead::Ended);
        core.start_next();
    });
}

async fn execute(
    s: &Rc<SessionShared>,
    kind: &ActionKind,
    cancel: &TurnCancel,
) -> Result<(), TurnError> {
    match kind {
        // The input is written before a recorded switch lands, so a switch
        // that fails or is stopped ends a turn that holds it.
        ActionKind::Send { content, comments, automation } => {
            let preamble = cancel.guard(s.runtime.preamble()).await?;
            s.update(|core| {
                let turn = core.turn();
                core.push_user(turn, content.clone(), comments.clone(), preamble, automation.clone());
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
        ActionKind::Resume { reason } => resume(s, cancel, reason.clone()).await,
        ActionKind::Edit => editing::run(s, cancel).await,
        ActionKind::Switch { target, to } => editing::switch(s, cancel, target, *to).await,
        ActionKind::Compact => {
            // The server compared the estimate with the share from which the
            // user may compact before it took the frame; the log repeats it.
            // A usage that cannot be read only leaves it out of the log: the
            // pass reads what it needs itself and reports its own failure.
            let usage = compaction::context_usage(s, cancel).await.ok();
            let trigger = compaction::Trigger::UserAction {
                estimate: usage.as_ref().map(|usage| usage.tokens),
                compact_from: usage.and_then(|usage| usage.compact_from),
            };
            compaction::compacting(s, compaction::run_pass(s, cancel, &trigger)).await?;
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
/// it.
async fn retry(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    let turn = {
        let _turn = s.persist_gate.acquire().await;
        // The input turn may have begun before the window, as when a
        // compaction ran inside it: the window then holds no block that
        // opens it, or only an agent message of a turn that began before.
        let rewind = loop {
            let found = s.read(|core| {
                let blocks = core.transcript.blocks();
                rewind(blocks)
                    .filter(|rewind| core.transcript.start() == 0 || opens_input_turn(&blocks[rewind.input]))
            });
            if let Some(found) = found {
                break found;
            }
            if !window::reach_further(s).await? {
                return Err(TurnError::refused(AgentCode::NothingToRetry, "There is no input turn to retry"));
            }
        };
        persist::commit_rewrite_held(s, rewind.retained).await?;
        rewind.turn
    };
    rerun(s, cancel, turn).await
}

/// Runs `turn` again from its input, which is the history's last turn.
async fn rerun(s: &Rc<SessionShared>, cancel: &TurnCancel, turn: TurnId) -> Result<(), TurnError> {
    s.update(|core| core.take_over_turn(turn));
    cancel.check()?;
    turn::apply_switch(s, cancel).await?;
    compaction::preflight(s, cancel).await?;
    turn::run(s, cancel).await
}

/// Finishes an unfinished turn.
///
/// It unwinds the turn to its resume point, keeping the compactions after it,
/// marks the stop it continues as resumed, appends a `resume` block and infers
/// again. A turn that left nothing but leftovers runs again from its input, as
/// a retry does.
async fn resume(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    reason: Option<String>,
) -> Result<(), TurnError> {
    let point = {
        let _turn = s.persist_gate.acquire().await;
        // The scan may pass the whole window, as one that holds a
        // compaction and leftovers: it goes on before it.
        let point = loop {
            let found = s.read(|core| {
                let blocks = core.transcript.blocks();
                resume_point_within(blocks).or_else(|| (core.transcript.start() == 0).then(|| resume_point(blocks)))
            });
            if let Some(found) = found {
                break found;
            }
            window::reach_further(s).await?;
        };
        // The unwind comes before a pending switch lands: a switch that
        // compacts would move the cut.
        let kept = s.read(|core| unwind(core.transcript.blocks(), point.cut));
        persist::commit_rewrite_held(s, kept).await?;
        point
    };
    if let Some(turn) = point.rerun {
        return rerun(s, cancel, turn).await;
    }
    // A stop that came during the save is recorded after the published
    // rewrite, and no `resume` block without a turn is left.
    cancel.check()?;
    s.update(|core| core.mark_abort_resumed());
    turn::apply_switch(s, cancel).await?;
    s.update(|core| core.push_resume(reason));
    compaction::preflight(s, cancel).await?;
    turn::run(s, cancel).await
}

/// Ends a switch of versions, unless dispose stopped it.
///
/// A switch that failed or was stopped changed nothing, so it leaves no stop
/// marker and no failure record. One that completed is answered as completed
/// whatever came after its commit. The closing save records the session idle
/// again.
async fn end_switch(
    s: &SessionShared,
    cancel: &TurnCancel,
    body: Result<(), TurnError>,
    reply: Option<ActionReply>,
) {
    cancel.acknowledge(s.read(|core| core.can_abort_again()));
    // The new path's usage reaches clients before the phase goes idle. A
    // stopped switch's token is cancelled, so the estimate reads with one
    // nothing stops.
    if body.is_ok() {
        compaction::report_context_usage(s, &TurnCancel::new()).await;
    }
    s.update(|core| core.end_action());
    let result = match (body, persist::flush(s).await) {
        (Ok(()), _) => Ok(ActionEnd::Completed),
        (Err(TurnError::Cancelled), _) => Ok(ActionEnd::Dropped),
        (Err(TurnError::Failed(report)), _) => Err(report),
    };
    if let Some(reply) = reply {
        // A caller that dropped its handle wants no answer.
        let _ = reply.send(result);
    }
}

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
