//! A turn (`runtime.md` § A turn): request the provider, apply its events
//! to the transcript as they stream, save, run the requested tools in order,
//! and ask again until a response requests no tool or a tool ends the turn.
//! Every wait on the provider, a hook or a tool is raced against the
//! action's stop and dropped when it comes; a save is never raced.

use std::rc::Rc;

use demi_core::TurnId;
use demi_provider::{InferenceRequest, ProviderEvent, ProviderRun};
use futures_util::StreamExt;

use super::{
    SessionShared, TurnError,
    cancel::TurnCancel,
    core::{SwitchPoint, TurnStage, with_request_id},
    persist,
    runtime::{ToolInvocation, ToolOutcome},
};
use crate::transcript::tool_input;

/// What one round of tools did.
#[derive(Debug, Default)]
struct ToolRound {
    executed: bool,
    stop_after_result: bool,
}

pub(super) async fn run(
    s: &Rc<SessionShared>,
    turn: &TurnId,
    cancel: &TurnCancel,
) -> Result<(), TurnError> {
    loop {
        cancel.check()?;
        apply_switch(s, SwitchPoint::Continuation).await;
        stream(s, turn, cancel).await?;
        cancel.check()?;
        let tools = run_tools(s, cancel).await?;
        if tools.stop_after_result || !tools.executed {
            return Ok(());
        }
    }
}

/// Lands a recorded model switch that lands at `point`, and closes the
/// runtimes it replaced: nothing runs on them any more.
pub(super) async fn apply_switch(s: &SessionShared, point: SwitchPoint) {
    let Some(switch) = s.update(|core| core.take_switch(point)) else {
        return;
    };
    let replaced = s.update(|core| core.install(switch));
    for mut runtime in replaced {
        runtime.close().await;
    }
}

/// One provider request, from building it to the end of its stream. The
/// runtime leaves its slot for the run and returns to it after.
async fn stream(
    s: &Rc<SessionShared>,
    turn: &TurnId,
    cancel: &TurnCancel,
) -> Result<(), TurnError> {
    s.update(|core| core.set_stage(TurnStage::Streaming));
    let request = request(s, turn, cancel).await?;
    let request_id = request.request_id.clone();
    let mut runtime = s
        .update(|core| core.provider.take())
        .expect("the provider runtime is in its slot between runs");
    let read = read(s, cancel, runtime.run(request), &request_id).await;
    s.update(|core| core.provider = Some(runtime));
    read?;
    s.update(|core| core.set_stage(TurnStage::Preparing));
    Ok(())
}

/// The next request: the context text first when the execution context
/// changed, saved at once, then the system prompt, the tools and the replay.
async fn request(
    s: &SessionShared,
    turn: &TurnId,
    cancel: &TurnCancel,
) -> Result<InferenceRequest, TurnError> {
    if let Some(text) = cancel.guard(s.runtime.context()).await? {
        s.update(|core| core.push_context(turn.clone(), text));
        persist::flush(s).await.map_err(TurnError::Store)?;
        cancel.check()?;
    }
    let system_prompt = cancel.guard(s.runtime.system_prompt()).await?;
    let tools = s.runtime.tools();
    let request_id = s.ids.next_id();
    Ok(s.read(|core| {
        core.inference_request(turn, system_prompt, tools, request_id, cancel.child_token())
    }))
}

/// Applies a run's events as they arrive. A thinking start waits until
/// something follows it, so that an empty lifecycle leaves nothing behind;
/// open answer text completes when anything but more text follows it.
async fn read(
    s: &SessionShared,
    cancel: &TurnCancel,
    mut events: ProviderRun<'_>,
    request_id: &str,
) -> Result<(), TurnError> {
    let mut thinking_started = false;
    while let Some(event) = cancel.guard(events.next()).await? {
        cancel.check()?;
        match event {
            ProviderEvent::ThinkingStart => thinking_started = true,
            ProviderEvent::Error(failure) => {
                let failure = with_request_id(failure, request_id);
                s.update(|core| core.record_failure(&failure));
                return Err(TurnError::Provider(Box::new(failure)));
            }
            event => {
                let completes_text = thinking_started
                    || !matches!(
                        event,
                        ProviderEvent::TextDelta(_) | ProviderEvent::ThinkingSignature(_)
                    );
                if completes_text {
                    complete_text(s).await;
                }
                s.update(|core| core.apply_event(event, thinking_started));
                thinking_started = false;
            }
        }
    }
    drop(events);
    complete_text(s).await;
    Ok(())
}

/// Marks open answer text at the end complete, in the session's save order,
/// so a command-state commit ordered before it is in its boundary.
async fn complete_text(s: &SessionShared) {
    if !s.read(|core| core.transcript.ends_with_open_text()) {
        return;
    }
    let _turn = s.persist_gate.acquire().await;
    s.update(|core| core.complete_tail_text());
}

/// Runs the calls the last response requested, one at a time in order,
/// after saving them as executing.
async fn run_tools(s: &SessionShared, cancel: &TurnCancel) -> Result<ToolRound, TurnError> {
    let calls = s.read(|core| core.transcript.pending_tool_calls());
    if calls.is_empty() {
        return Ok(ToolRound::default());
    }
    s.update(|core| core.set_stage(TurnStage::Tools));
    // Every call is in the store as executing before a tool runs: a process
    // that dies during one leaves it executing, and restore completes it as
    // interrupted without running it again.
    persist::flush(s).await.map_err(TurnError::Store)?;
    let tools = s.runtime.tools();
    let mut round = ToolRound {
        executed: true,
        stop_after_result: false,
    };
    for call in calls {
        cancel.check()?;
        let outcome = if tools.iter().any(|tool| tool.name == call.tool_name) {
            let invocation = ToolInvocation {
                tool_use_id: call.tool_use_id.clone(),
                tool_name: call.tool_name.clone(),
                input: tool_input(&call.input),
            };
            match cancel.guard(s.runtime.invoke_tool(invocation)).await? {
                Ok(outcome) => outcome,
                Err(failure) => ToolOutcome::error(format!("Tool failed: {}", failure.0)),
            }
        } else {
            ToolOutcome::error(format!("Tool not found: {}", call.tool_name))
        };
        round.stop_after_result |= outcome.stop_after_result;
        s.update(|core| core.complete_tool_call(&call.tool_use_id, outcome));
    }
    s.update(|core| core.set_stage(TurnStage::Preparing));
    Ok(round)
}
