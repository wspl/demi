//! The action worker (`runtime.md` § Actions, § A turn): one task per
//! session runs one action at a time. An action holds the tree's admission
//! while it runs, records how it ended, saves its checkpoint once the session
//! is idle again, and then starts the next waiting action.

use std::rc::{Rc, Weak};

use demi_core::TurnId;
use tokio::sync::Notify;

use super::{
    ActionEnd, ErrorReport, SessionEvent, SessionShared, Settle, TurnError,
    cancel::{CancelReason, TurnCancel},
    core::{ActionKind, StartedAction, SwitchPoint},
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
        Ok(_) => execute(s, &kind, &turn, &cancel).await,
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
    let outcome = conclude(s, &cancel, body);
    cancel.acknowledge(s.read(|core| core.can_abort_again()));
    s.update(|core| core.end_action());
    // The save after the phase is idle: a checkpoint that says an action was
    // running is one the process died in.
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
    turn: &TurnId,
    cancel: &TurnCancel,
) -> Result<(), TurnError> {
    match kind {
        ActionKind::Send { content, .. } => {
            // The version current when the turn started, before its hooks
            // and commands run.
            let revision = s.read(|core| core.commands.revision());
            turn::apply_switch(s, SwitchPoint::ActionStart).await;
            let preamble = cancel.guard(s.runtime.preamble()).await?;
            s.update(|core| core.push_user(turn.clone(), content.clone(), preamble, revision));
            turn::run(s, turn, cancel).await
        }
    }
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
        Err(TurnError::Provider(failure)) => {
            let report = ErrorReport::from(failure.as_ref());
            s.update(|core| core.fail(&report));
            Outcome::Failed(report)
        }
        Err(TurnError::Store(error)) => {
            let report = ErrorReport::from(error);
            s.update(|core| core.fail(&report));
            Outcome::Failed(report)
        }
    }
}
