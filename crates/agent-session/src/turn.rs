//! A turn (`runtime.md` § A turn): write waiting input, compact first when
//! the request reaches a size threshold, request the provider, retry what
//! waiting can fix and compact once after a refusal as too large, apply its
//! events to the transcript as they stream, save, run the requested tools in
//! steps, compact when the usage reached the threshold, and ask again until a
//! response requests no tool, unless input arrived during the round. Every
//! wait on the provider, a hook or a tool is raced
//! against the action's stop and dropped when it comes; a save is never
//! raced. A message the user sends now cuts a provider request and a step's
//! calls short without stopping anything (`runtime.md` § Send now).

use std::rc::Rc;

use demi_agent_store::media;
use demi_agent_transcript::{
    PendingCall,
    estimate::{context_tokens, request_size},
    replay_start, resume_point, tool_input,
};
use demi_provider_common::{
    ErrorCode, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun,
};
use demi_shared_types::{Block, ModelSelection};
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::{
    ErrorReport, SessionEvent, SessionShared, TurnError,
    cancel::TurnCancel,
    compaction,
    core::{SendNow, SessionCore, TurnStage, with_request_id},
    input::Take,
    media::model_view,
    persist, reports,
    runtime::{InputArrival, SeenContext, ToolEffect, ToolInvocation, ToolOutcome},
};

/// How many compactions one turn runs after responses over the threshold.
const MAX_AUTO_COMPACTIONS: u32 = 3;


pub(super) async fn run(s: &Rc<SessionShared>, cancel: &TurnCancel) -> Result<(), TurnError> {
    run_turn(s, cancel, true).await
}

/// An accepted edit's replacement turn: its first request carries the model
/// it was prepared with, and a switch that waited for the edit lands at the
/// next continuation boundary (`runtime.md` § Model switch).
pub(super) async fn run_replacement(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
) -> Result<(), TurnError> {
    run_turn(s, cancel, false).await
}

/// The turn's requests and tool rounds; a recorded switch lands before each
/// request, except the first when `switch_first` is false.
async fn run_turn(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    mut switch_first: bool,
) -> Result<(), TurnError> {
    let mut auto_compactions = 0;
    // The turn's first request follows its new input; each later one
    // continues the running turn.
    let mut continues = false;
    loop {
        cancel.check()?;
        if switch_first && apply_switch(s, cancel).await? {
            s.update(|core| core.push_resume());
        }
        switch_first = true;
        // A message sent now during a compaction or a save ends the turn
        // once that is done.
        if ends_for_message(s).await? {
            return Ok(());
        }
        write_inputs(s).await?;
        let before = s.read(|core| core.inputs.arrivals());
        let recover = stream(s, cancel, continues).await?;
        continues = true;
        cancel.check()?;
        if !recover {
            write_inputs_since(s, before).await?;
        }
        let executed = run_tools(s, cancel, recover).await?;
        if ends_for_message(s).await? {
            return Ok(());
        }
        if recover && auto_compactions < MAX_AUTO_COMPACTIONS {
            let before_compaction = estimate(s, cancel).await?;
            let compacted = compaction::compacting(s, compaction::run_pass(s, cancel)).await?;
            // Looping on a pass that freed nothing would summarize its own
            // summaries and pile up `resume` blocks.
            if compacted && estimate(s, cancel).await? < before_compaction {
                auto_compactions += 1;
                s.update(|core| core.push_resume());
                continue;
            }
        }
        if s.read(|core| core.inputs.arrivals()) > before {
            write_inputs_since(s, before).await?;
            continue;
        }
        if !executed {
            return Ok(());
        }
    }
}

/// The estimate of the next request of the current model over the replayed
/// blocks, as the model receives them.
async fn estimate(s: &SessionShared, cancel: &TurnCancel) -> Result<u64, TurnError> {
    let view = model_view(s, cancel).await?;
    Ok(s.read(|core| context_tokens(&core.request_view(&view))))
}

/// Lands the recorded model switch: the history is first compacted to fit
/// the new model with the current one, then the switch becomes current and
/// the runtimes it replaced are closed. Returns whether it compacted; a
/// switch whose compaction failed stays recorded.
pub(super) async fn apply_switch(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
) -> Result<bool, TurnError> {
    let Some((target, limits)) = s.read(|core| core.switch_target()) else {
        return Ok(false);
    };
    let compacted = compaction::compact_to_fit(s, &target, limits, cancel).await?;
    let replaced = s.update(|core| core.install_switch());
    for mut runtime in replaced {
        runtime.close().await;
    }
    Ok(compacted)
}

/// Writes the waiting input into the turn: human steers, command reports
/// and agent messages. An agent message written is saved at once.
async fn write_inputs(s: &SessionShared) -> Result<(), TurnError> {
    if s.update(|core| core.write_inputs(Take::Everything)) {
        persist::flush(s).await?;
    }
    Ok(())
}

/// Takes what the user sent now at this boundary (`runtime.md` § Send now):
/// a queued message ends the turn here, once all waiting input is written,
/// and true says so; a steer goes on as any input that arrived.
async fn ends_for_message(s: &SessionShared) -> Result<bool, TurnError> {
    match s.update(SessionCore::take_send_now) {
        Some(SendNow::Message) => {
            write_inputs(s).await?;
            Ok(true)
        }
        Some(SendNow::Steer) | None => Ok(false),
    }
}

/// Resolves once the user sent a message now (`runtime.md` § Send now),
/// which cuts a provider request short.
async fn sent_now(s: &SessionShared) {
    let mut arrivals = s.arrivals.subscribe();
    if arrivals.wait_for(|arrivals| arrivals.send_now).await.is_err() {
        // The sender lives in `s`, which outlives this wait.
        std::future::pending::<()>().await;
    }
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
/// unwound (`failures-and-recovery.md` § Retries). A request that reaches a
/// size threshold of its vendor compacts first, and one refused as too
/// large compacts once and goes again (`compaction.md` § When compaction
/// runs); `continues` says it continues a running turn, so a pass is
/// followed by a `resume` block. Returns whether a response's usage reached
/// the compaction threshold. The stage stays streaming across the waits, so
/// steers keep being accepted. A message the user sent now cuts the stream,
/// or the wait before a retry, short: what streamed stays.
async fn stream(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    continues: bool,
) -> Result<bool, TurnError> {
    s.update(|core| core.set_stage(TurnStage::Streaming));
    let policy = s.config.retry;
    let mut attempt = 1;
    let mut size_checked = false;
    let mut refusal_compacted = false;
    loop {
        let mut request = request(s, cancel).await?;
        // A pass that leaves the request as large is not repeated: the
        // request goes out.
        if !size_checked {
            size_checked = true;
            let limits = s.read(|core| core.request_limits());
            let size = request_size(&request.system_prompt, &request.items);
            if s.config.compaction.size_reached(limits, size)
                && compact_before_request(s, cancel, continues).await?
            {
                request = self::request(s, cancel).await?;
            }
        }
        let request_id = request.request_id.clone();
        let start = s.read(|core| core.transcript.blocks().len());
        let model = s.read(|core| core.model.clone());
        let window = cancel.guard(compaction::window_in_use(s, &model)).await?;
        let mut runtime = s
            .update(|core| core.take_runtime())
            .expect("the provider runtime is in its slot between runs");
        let request_cancel = request.cancel.clone();
        let read = read(s, cancel, window, runtime.run(request), &request_cancel).await;
        // However the request ended, no call of it is being written.
        s.update(|core| core.end_writing());
        s.return_runtime(runtime);
        let failure = match read? {
            Ok(recover) => {
                s.update(|core| core.set_stage(TurnStage::Preparing));
                compaction::report_context_usage(s, cancel).await;
                return Ok(recover);
            }
            Err(failure) => with_request_id(failure, &request_id),
        };
        let unwindable = s.read(|core| resume_point(core.transcript.blocks()).cut <= start);
        // The same request would be refused again: one pass, then the
        // request built from the compacted history.
        let too_large = failure.code == Some(ErrorCode::ContextLengthExceeded);
        if too_large && unwindable && !refusal_compacted && s.config.compaction.automatic() {
            refusal_compacted = true;
            cut_history(s, start).await?;
            if compact_before_request(s, cancel, continues).await? {
                continue;
            }
        }
        if !(unwindable && policy.retries(attempt, &failure)) {
            let report = ErrorReport::from(&failure);
            s.update(|core| core.record_failure(&report));
            return Err(TurnError::Failed(Box::new(report)));
        }
        // The attempt's leftovers go.
        cut_history(s, start).await?;
        let delay = policy.delay(attempt, failure.retry_after);
        s.emit(SessionEvent::RetryScheduled {
            attempt,
            delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            code: failure.code.as_ref().map(|code| code.as_str().to_owned()),
            diagnostics: failure.diagnostics.map(|diagnostics| *diagnostics),
        });
        tokio::select! {
            biased;
            () = sent_now(s) => {
                s.update(|core| core.set_stage(TurnStage::Preparing));
                return Ok(false);
            }
            slept = cancel.guard(tokio::time::sleep(delay)) => slept?,
        }
        attempt += 1;
    }
}

/// One compaction pass before a request; a pass that compacted inside a
/// running turn appends a `resume` block, while before a turn's first
/// request its new input is last already. Returns whether it compacted.
async fn compact_before_request(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    continues: bool,
) -> Result<bool, TurnError> {
    let compacted = compaction::compacting(s, compaction::run_pass(s, cancel)).await?;
    if compacted && continues {
        s.update(|core| core.push_resume());
    }
    Ok(compacted)
}

/// Cuts the history to its first `cut` blocks.
pub(super) async fn cut_history(s: &SessionShared, cut: usize) -> Result<(), TurnError> {
    let blocks = s.read(|core| core.transcript.blocks()[..cut].to_vec());
    persist::commit_rewrite(s, blocks).await?;
    Ok(())
}

/// The next request: what the context sources tell the node first, saved at
/// once, then the system prompt, the tools and the replay.
async fn request(s: &SessionShared, cancel: &TurnCancel) -> Result<InferenceRequest, TurnError> {
    // Only the context the model receives counts as seen: a block before
    // the last compaction boundary is summarized away, so its source tells
    // the model again (`runtime.md` § Context).
    let (seen, turn) = s.read(|core| {
        let blocks = core.transcript.blocks();
        let seen: Vec<(String, String)> = blocks[replay_start(blocks)..]
            .iter()
            .filter_map(|block| match block {
                Block::Context(context) => Some((context.source.clone(), context.text.clone())),
                _ => None,
            })
            .collect();
        (seen, core.turn())
    });
    let seen: Vec<SeenContext<'_>> = seen
        .iter()
        .map(|(source, text)| SeenContext { source, text })
        .collect();
    let news = cancel.guard(s.runtime.context(&seen, &turn)).await?;
    if !news.is_empty() {
        s.update(|core| {
            for news in news {
                core.push_context(news);
            }
        });
        persist::flush(s).await?;
        cancel.check()?;
    }
    let model = s.read(|core| core.model.clone());
    let system_prompt = system_prompt(s, &model, cancel).await?;
    let request_id = s.ids.next_id();
    let view = model_view(s, cancel).await?;
    Ok(s.update(|core| {
        core.inference_request(
            &view,
            system_prompt,
            request_id,
            cancel.child_token(),
        )
    }))
}

/// The system prompt of a request that infers with `model`. One that cannot
/// be rendered, as when the model's provider entry is gone, fails the
/// request before any vendor, recorded as a failed request is.
pub(super) async fn system_prompt(
    s: &SessionShared,
    model: &ModelSelection,
    cancel: &TurnCancel,
) -> Result<String, TurnError> {
    match cancel.guard(s.runtime.system_prompt(model)).await? {
        Ok(prompt) => Ok(prompt),
        Err(message) => {
            let report = ErrorReport {
                message,
                code: None,
                diagnostics: None,
            };
            s.update(|core| core.record_failure(&report));
            Err(TurnError::Failed(Box::new(report)))
        }
    }
}

/// Applies a run's events as they arrive. A thinking start waits until
/// something follows it, so that an empty lifecycle leaves nothing behind;
/// open answer text completes when anything but more text follows it. The
/// run's failure is handed back unrecorded, for the retry to decide on; the
/// inner result says whether a response's usage reached the compaction
/// threshold of `window`, the window in use for the run's model. A message
/// the user sent now ends it where it is, cancelling the request through
/// `request_cancel`, as a Stop would, and what streamed stays.
async fn read(
    s: &SessionShared,
    cancel: &TurnCancel,
    window: Option<u64>,
    mut events: ProviderRun<'_>,
    request_cancel: &CancellationToken,
) -> Result<Result<bool, ProviderFailure>, TurnError> {
    let mut thinking_started = false;
    let mut recover = false;
    let mut cut = std::pin::pin!(sent_now(s));
    loop {
        let event = tokio::select! {
            biased;
            () = &mut cut => {
                request_cancel.cancel();
                break;
            }
            event = cancel.guard(events.next()) => event?,
        };
        let Some(event) = event else {
            break;
        };
        cancel.check()?;
        match event {
            ProviderEvent::ThinkingStart => thinking_started = true,
            ProviderEvent::Error(failure) => return Ok(Err(failure)),
            // Entries write no block: open text stays open, and a thinking
            // start still waits for what follows it.
            ProviderEvent::Entries { of, entries } => {
                s.update(|core| core.keep_entries(of, entries));
            }
            event => {
                let completes_text = thinking_started
                    || !matches!(
                        event,
                        ProviderEvent::TextDelta(_) | ProviderEvent::ThinkingSignature(_)
                    );
                if completes_text {
                    complete_text(s);
                }
                if let ProviderEvent::Response(usage) = &event {
                    recover |= s.config.compaction.reached(window, usage);
                }
                s.update(|core| core.apply_event(event, thinking_started));
                thinking_started = false;
            }
        }
    }
    drop(events);
    complete_text(s);
    Ok(Ok(recover))
}

/// Marks open answer text at the end complete, so a Fork may end at it
/// (`conversation-fork.md` § Eligibility).
fn complete_text(s: &SessionShared) {
    s.update(|core| core.complete_tail_text());
}

/// Runs the calls the last response requested, after saving them as
/// executing, in steps (`runtime.md` § Dispatch and failures): consecutive
/// calls of a tool that runs together start together, and each other call
/// is a step of its own. A step ends when each of its calls has returned;
/// each result is recorded as its call returns, in its call's place, and a
/// Stop keeps those of the calls that returned before it. Input that arrives
/// meanwhile is written after each step, unless the turn is about to
/// compact. Returns whether the response requested any call.
async fn run_tools(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    defer_input: bool,
) -> Result<bool, TurnError> {
    let calls = s.read(|core| core.transcript.pending_tool_calls());
    if calls.is_empty() {
        return Ok(false);
    }
    s.update(|core| core.set_stage(TurnStage::Tools));
    // Every call is in the store as executing before a tool runs: a process
    // that dies during one leaves it executing, and restore completes it as
    // interrupted without running it again.
    persist::flush(s).await?;
    let tools = s.runtime.tools();
    let mut calls = calls.into_iter().peekable();
    while let Some(first) = calls.next() {
        cancel.check()?;
        if s.read(SessionCore::send_now_pending) {
            // The user sent a message now: no other call starts
            // (`runtime.md` § Send now).
            for call in std::iter::once(first).chain(calls.by_ref()) {
                record_result(s, &call, ToolOutcome::not_run()).await;
            }
            break;
        }
        let mut step = vec![first];
        if s.runtime.runs_together(&step[0].tool_name) {
            while let Some(next) = calls.next_if(|call| call.tool_name == step[0].tool_name) {
                step.push(next);
            }
        }
        let (before, joined) = s.read(|core| (core.inputs.arrivals(), core.inputs.joining()));
        let ran = if tools.iter().any(|tool| tool.name == step[0].tool_name) {
            let (model, request_limits) =
                s.read(|core| (core.model.clone(), core.request_limits()));
            let invocations = step
                .iter()
                .map(|call| ToolInvocation {
                    tool_use_id: call.tool_use_id.clone(),
                    tool_name: call.tool_name.clone(),
                    input: tool_input(&call.input),
                    model: model.clone(),
                    request_limits,
                    arrival: InputArrival::new(s.arrivals.subscribe(), joined),
                })
                .collect();
            let mut returned = s.runtime.invoke_step(invocations);
            // Each result is recorded as its call returns, so the page shows
            // a call's end, its changed files among it, while the step's
            // other calls run. Only the wait is cancelled: a Stop during the
            // step ends the calls still running, and one that returned keeps
            // its result, even one that returned as the Stop came.
            loop {
                match cancel.guard(returned.next()).await {
                    Ok(Some((index, outcome))) => {
                        let outcome = outcome.unwrap_or_else(|failure| {
                            ToolOutcome::error(format!("Tool failed: {}", failure.0))
                        });
                        record_result(s, &step[index], outcome).await;
                    }
                    Ok(None) => break Ok(()),
                    Err(stopped) => break Err(stopped),
                }
            }
        } else {
            for call in &step {
                let outcome = ToolOutcome::error(format!("Tool not found: {}", call.tool_name));
                record_result(s, call, outcome).await;
            }
            Ok(())
        };
        ran?;
        if !defer_input {
            write_inputs_since(s, before).await?;
        }
    }
    s.update(|core| core.set_stage(TurnStage::Preparing));
    Ok(true)
}

/// Records the result of `call`, and watches the command it left running;
/// its media are stored before it enters the transcript, and the session
/// holds their bytes (`runtime.md` § Media).
async fn record_result(s: &Rc<SessionShared>, call: &PendingCall, outcome: ToolOutcome) {
    let (output, held) = media::store_result(outcome.output, s.store.blobs()).await;
    s.update(|core| {
        core.media.absorb(held);
        core.complete_tool_call(&call.tool_use_id, output, outcome.is_error, outcome.view);
    });
    if let Some(ToolEffect::Background {
        command,
        interval_ms,
        title,
    }) = outcome.effect
    {
        s.update(|core| core.watch_command(command.clone(), interval_ms, title));
        reports::start_watch(s, &command);
    }
}
