//! A turn (`runtime.md` § A turn): write waiting input, request the
//! provider and retry what waiting can fix, apply its events to the
//! transcript as they stream, save, run the requested tools in order, compact
//! when the usage reached the threshold, and ask again until a response
//! requests no tool or a tool ends the turn, unless input arrived during the
//! round. Every wait on the provider, a hook or a tool is raced against the
//! action's stop and dropped when it comes; a save is never raced.

use std::rc::Rc;

use demi_core::{BlockId, ToolResultContentBlock, ToolView, WakeupId};
use demi_provider::{InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun};
use futures_util::StreamExt;

use super::{
    SessionEvent, SessionShared, TurnError,
    cancel::TurnCancel,
    compaction,
    core::{SwitchPoint, TurnStage, with_request_id},
    input::Take,
    persist,
    runtime::{ToolEffect, ToolInvocation, ToolOutcome},
};
use crate::{
    store::BoundaryEdge,
    transcript::{estimate::context_tokens, resume_point, tool_input},
};

/// How many compactions one turn runs after responses over the threshold.
const MAX_AUTO_COMPACTIONS: u32 = 3;

/// What one round of tools did.
#[derive(Debug, Default)]
struct ToolRound {
    executed: bool,
    stop_after_result: bool,
}

pub(super) async fn run(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    let mut auto_compactions = 0;
    loop {
        cancel.check()?;
        if apply_switch(s, SwitchPoint::Continuation, cancel).await? {
            s.update(|core| core.push_resume());
        }
        write_inputs(s).await?;
        let before = s.read(|core| core.inputs.arrivals());
        let recover = stream(s, cancel).await?;
        cancel.check()?;
        if !recover {
            write_inputs_since(s, before).await?;
        }
        let tools = run_tools(s, cancel, recover).await?;
        if recover && auto_compactions < MAX_AUTO_COMPACTIONS {
            let estimate =
                |s: &SessionShared| s.read(|core| context_tokens(core.transcript.blocks(), None));
            let before_compaction = estimate(s);
            let compacted = compaction::compacting(s, compaction::run_pass(s, cancel)).await?;
            // Looping on a pass that freed nothing would summarize its own
            // summaries and pile up `resume` blocks.
            if compacted && estimate(s) < before_compaction {
                auto_compactions += 1;
                s.update(|core| core.push_resume());
                continue;
            }
        }
        if s.read(|core| core.inputs.arrivals()) > before {
            write_inputs_since(s, before).await?;
            continue;
        }
        if tools.stop_after_result || !tools.executed {
            return Ok(());
        }
    }
}

/// Lands the recorded model switch when it lands at `point`: the history is
/// first compacted to fit the new model with the current one, then the switch
/// becomes current and the runtimes it replaced are closed. Returns whether it
/// compacted; a switch whose compaction failed stays recorded.
pub(super) async fn apply_switch(
    s: &Rc<SessionShared>,
    point: SwitchPoint,
    cancel: &TurnCancel,
) -> Result<bool, TurnError> {
    let Some(target) = s.read(|core| core.switch_target(point)) else {
        return Ok(false);
    };
    let compacted = compaction::compact_to_fit(s, &target, cancel).await?;
    let replaced = s.update(|core| core.install_switch());
    for mut runtime in replaced {
        runtime.close().await;
    }
    Ok(compacted)
}

/// Writes the waiting input into the turn: human steers, fired wakeups and
/// agent messages. An agent message written is saved at once.
async fn write_inputs(s: &SessionShared) -> Result<(), TurnError> {
    if s.update(|core| core.write_inputs(Take::Everything)) {
        persist::flush(s).await?;
    }
    Ok(())
}

/// Writes the waiting input when some arrived since `arrivals` was read.
async fn write_inputs_since(s: &SessionShared, arrivals: u64) -> Result<(), TurnError> {
    if s.read(|core| core.inputs.arrivals()) > arrivals {
        write_inputs(s).await?;
    }
    Ok(())
}

/// One provider request, from building it to the end of its stream,
/// retrying transient failures while everything the attempt wrote can be
/// unwound (`failures-and-recovery.md` § Retries). Returns whether a
/// response's usage reached the compaction threshold. The stage stays
/// streaming across the waits, so steers keep being accepted.
async fn stream(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<bool, TurnError> {
    s.update(|core| core.set_stage(TurnStage::Streaming));
    let policy = s.config.retry;
    let mut attempt = 1;
    loop {
        let request = request(s, cancel).await?;
        let request_id = request.request_id.clone();
        let start = s.read(|core| core.transcript.blocks().len());
        let mut runtime = s
            .update(|core| core.provider.take())
            .expect("the provider runtime is in its slot between runs");
        let read = read(s, cancel, runtime.run(request)).await;
        s.return_runtime(runtime);
        let failure = match read? {
            Ok(recover) => {
                s.update(|core| core.set_stage(TurnStage::Preparing));
                return Ok(recover);
            }
            Err(failure) => with_request_id(failure, &request_id),
        };
        let unwindable = s.read(|core| resume_point(core.transcript.blocks()).cut <= start);
        if !(unwindable && policy.retries(attempt, &failure)) {
            s.update(|core| core.record_failure(&failure));
            return Err(TurnError::Failed(Box::new((&failure).into())));
        }
        // The attempt's leftovers go, and command state returns to where the
        // attempt began.
        restore_command_state(s, start).await?;
        let delay = policy.delay(attempt, failure.retry_after);
        s.emit(SessionEvent::RetryScheduled {
            attempt,
            delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            code: failure.code.as_ref().map(|code| code.as_str().to_owned()),
            diagnostics: failure.diagnostics,
        });
        cancel.guard(tokio::time::sleep(delay)).await?;
        attempt += 1;
    }
}

/// Cuts the history to its first `cut` blocks, with the command state
/// recorded after the last block kept, or the empty version when none is.
pub(super) async fn restore_command_state(s: &SessionShared, cut: usize) -> Result<(), TurnError> {
    let (blocks, revision) = s.read(|core| {
        let blocks = core.transcript.blocks()[..cut].to_vec();
        let revision = match blocks.last() {
            Some(last) => core
                .commands
                .boundary(last.id(), BoundaryEdge::AfterBlock)
                .ok_or_else(|| missing_boundary(last.id())),
            None => Ok(0),
        };
        (blocks, revision)
    });
    persist::commit_rewrite(s, blocks, revision?).await?;
    Ok(())
}

fn missing_boundary(block: &BlockId) -> TurnError {
    TurnError::refused(format!("No command-state boundary after block {block}"))
}

/// The next request: the context text first when the execution context
/// changed, saved at once, then the system prompt, the tools and the replay.
async fn request(s: &SessionShared, cancel: &TurnCancel) -> Result<InferenceRequest, TurnError> {
    if let Some(text) = cancel.guard(s.runtime.context()).await? {
        s.update(|core| core.push_context(text));
        persist::flush(s).await?;
        cancel.check()?;
    }
    let system_prompt = cancel.guard(s.runtime.system_prompt()).await?;
    let tools = s.runtime.tools();
    let request_id = s.ids.next_id();
    Ok(s.read(|core| {
        core.inference_request(system_prompt, tools, request_id, cancel.child_token())
    }))
}

/// Applies a run's events as they arrive. A thinking start waits until
/// something follows it, so that an empty lifecycle leaves nothing behind;
/// open answer text completes when anything but more text follows it. The
/// run's failure is handed back unrecorded, for the retry to decide on; the
/// inner result says whether a response's usage reached the compaction
/// threshold.
async fn read(
    s: &SessionShared,
    cancel: &TurnCancel,
    mut events: ProviderRun<'_>,
) -> Result<Result<bool, ProviderFailure>, TurnError> {
    let mut thinking_started = false;
    let mut recover = false;
    while let Some(event) = cancel.guard(events.next()).await? {
        cancel.check()?;
        match event {
            ProviderEvent::ThinkingStart => thinking_started = true,
            ProviderEvent::Error(failure) => return Ok(Err(failure)),
            event => {
                let completes_text = thinking_started
                    || !matches!(
                        event,
                        ProviderEvent::TextDelta(_) | ProviderEvent::ThinkingSignature(_)
                    );
                if completes_text {
                    complete_text(s).await;
                }
                if let ProviderEvent::Response(usage) = &event {
                    let window = s.read(|core| core.model.model.context_window);
                    recover |= s.config.compaction.reached(window, usage);
                }
                s.update(|core| core.apply_event(event, thinking_started));
                thinking_started = false;
            }
        }
    }
    drop(events);
    complete_text(s).await;
    Ok(Ok(recover))
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
/// after saving them as executing. Input that arrives meanwhile is written
/// after each call, unless the turn is about to compact.
async fn run_tools(
    s: &SessionShared,
    cancel: &TurnCancel,
    defer_input: bool,
) -> Result<ToolRound, TurnError> {
    let calls = s.read(|core| core.transcript.pending_tool_calls());
    if calls.is_empty() {
        return Ok(ToolRound::default());
    }
    s.update(|core| core.set_stage(TurnStage::Tools));
    // Every call is in the store as executing before a tool runs: a process
    // that dies during one leaves it executing, and restore completes it as
    // interrupted without running it again.
    persist::flush(s).await?;
    let tools = s.runtime.tools();
    let mut round = ToolRound {
        executed: true,
        stop_after_result: false,
    };
    for call in calls {
        cancel.check()?;
        let before = s.read(|core| core.inputs.arrivals());
        let outcome = if tools.iter().any(|tool| tool.name == call.tool_name) {
            let invocation = ToolInvocation {
                tool_use_id: call.tool_use_id.clone(),
                tool_name: call.tool_name.clone(),
                input: tool_input(&call.input),
                model: s.read(|core| core.model.clone()),
                generation: s.read(|core| core.generation.number),
                cancel: cancel.child_token(),
            };
            match cancel.guard(s.runtime.invoke_tool(invocation)).await? {
                Ok(outcome) => outcome,
                Err(failure) => ToolOutcome::error(format!("Tool failed: {}", failure.0)),
            }
        } else {
            ToolOutcome::error(format!("Tool not found: {}", call.tool_name))
        };
        let outcome = match outcome.effect {
            Some(ToolEffect::ScheduleYield { duration_ms }) => {
                round.stop_after_result = true;
                s.update(|core| yield_result(core.schedule_wakeup(duration_ms), duration_ms))
            }
            None => outcome,
        };
        s.update(|core| core.complete_tool_call(&call.tool_use_id, outcome));
        if !defer_input {
            write_inputs_since(s, before).await?;
        }
    }
    s.update(|core| core.set_stage(TurnStage::Preparing));
    Ok(round)
}

/// The result of a `yield` call, which the session writes because the
/// wakeup's id is its own.
fn yield_result(wakeup_id: WakeupId, duration_ms: u32) -> ToolOutcome {
    let text = format!("yield scheduled\nwakeupId: {wakeup_id}\ndurationMs: {duration_ms}");
    ToolOutcome {
        output: vec![ToolResultContentBlock::Text { text }],
        is_error: false,
        view: Some(ToolView::YieldWakeup {
            wakeup_id,
            duration_ms,
        }),
        effect: None,
    }
}
