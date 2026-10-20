//! A turn (`runtime.md` § A turn): write waiting input, request the
//! provider, retry what waiting can fix and compact once after a refusal for
//! its tokens, apply its events to the transcript as they stream, save, run
//! the requested tools in steps, compact when the usage reached the
//! threshold, and ask again until a response requests no tool, unless input
//! arrived during the round. A request's size never compacts. Every
//! wait on the provider, a hook or a tool is raced
//! against the action's stop and dropped when it comes; a save is never
//! raced. A message the user sends now cuts a provider request and a step's
//! calls short without stopping anything (`runtime.md` § Send now).

use std::rc::Rc;

use demi_agent_store::{Kind, StoreError, changes, media};
use demi_agent_transcript::{
    PendingCall,
    estimate::context_tokens,
    resume_point_within, tool_input,
};
use demi_provider_common::{
    ErrorCode, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun,
};
use demi_shared_types::{Block, BlockId, ModelSelection, ShellToolView, ToolView};
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use demi_conversation_socket_protocol::AgentCode;

use super::{
    ErrorReport, SessionEvent, SessionShared, TurnError,
    cancel::{CancelReason, TurnCancel},
    compaction::{self, Trigger},
    core::{SendNow, SessionCore, TurnStage, with_request_id},
    input::Take,
    media::model_view,
    persist, reports,
    runtime::{
        CallCommand, InputArrival, SeenContext, SystemPrompt, ToolEffect, ToolInvocation,
        ToolOutcome,
    },
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
        // The turn needs the model's next answer from here: after its input,
        // a step's calls, or a response that the input arriving meanwhile
        // continues (`rendering.md` § Waiting for the model).
        s.update(SessionCore::start_waiting);
        if switch_first && apply_switch(s, cancel).await? {
            s.update(|core| core.push_resume(None));
        }
        switch_first = true;
        // A message sent now during a compaction or a save ends the turn
        // once that is done.
        if ends_for_message(s).await? {
            return Ok(());
        }
        write_inputs(s).await?;
        let before = s.read(|core| core.inputs.arrivals());
        let reached = stream(s, cancel, continues).await?;
        let recover = reached.is_some();
        continues = true;
        cancel.check()?;
        // A session copy's answer is its summary: a call in it never runs
        // (`compaction.md` § The summary request).
        if s.read(|core| core.one_request) {
            s.release_events();
            return Ok(());
        }
        let arrived = s.update(|core| core.arrived_since(before));
        // A response that requested no tool held its events for the turn's
        // end (`read`); a turn that goes on to compact or to the input that
        // arrived meanwhile shows them now.
        if recover || arrived {
            s.release_events();
        }
        if !recover && arrived {
            // The command reports wait until the response's tools have run;
            // the boundary after them writes them. The human steers wait for
            // the next request, pending, so Send Now and Recall still reach
            // them while the tools run.
            if s.update(|core| core.write_inputs(Take::AgentMessages)) {
                persist::flush(s).await?;
            }
        }
        let executed = run_tools(s, cancel, recover).await?;
        if ends_for_message(s).await? {
            return Ok(());
        }
        if let Some(trigger) = reached.filter(|_| auto_compactions < MAX_AUTO_COMPACTIONS) {
            let before_compaction = estimate(s, cancel).await?;
            let compacted =
                compaction::compacting(s, compaction::run_pass(s, cancel, &trigger)).await?;
            // Looping on a pass that freed nothing would summarize its own
            // summaries and pile up `resume` blocks.
            if compacted && estimate(s, cancel).await? < before_compaction {
                auto_compactions += 1;
                s.update(|core| core.push_resume(None));
                continue;
            }
        }
        // Input that arrived during the round asks the provider once more;
        // the human steers among it are written before that request.
        if s.update(|core| core.arrived_since(before)) {
            write_inputs_after_step(s).await?;
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
    let model = s.read(|core| core.model.clone());
    let system_prompt = system_prompt(s, &model, cancel).await?.text();
    let view = model_view(s, cancel).await?;
    Ok(s.read(|core| context_tokens(&core.request_view(&view), &system_prompt)))
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
    write_taken(s, Take::Everything).await
}

/// Writes the waiting input but the human steers, which stay pending until
/// the request that carries them (`runtime.md` § Input).
async fn write_inputs_after_step(s: &SessionShared) -> Result<(), TurnError> {
    write_taken(s, Take::AllButSteers).await
}

async fn write_taken(s: &SessionShared, take: Take) -> Result<(), TurnError> {
    if s.update(|core| core.write_inputs(take)) {
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

/// Writes the waiting input but the human steers when some arrived since
/// `arrivals` was read: the boundary after a step of tool calls.
async fn write_inputs_since(s: &SessionShared, arrivals: u64) -> Result<(), TurnError> {
    if s.update(|core| core.arrived_since(arrivals)) {
        write_inputs_after_step(s).await?;
    }
    Ok(())
}

/// One provider request, from building it to the end of its stream,
/// retrying transient failures while everything the attempt wrote can be
/// unwound (`failures-and-recovery.md` § Retries). A request refused for
/// its tokens compacts once and goes again (`compaction.md` § When
/// compaction runs); one refused for its size does not. `continues` says it
/// continues a running turn, so a pass is followed by a `resume` block.
/// Returns the trigger of a pass when the usage of a response that
/// requested a tool reached the compaction threshold. The stage stays
/// streaming across the waits, so steers keep being accepted. A message the
/// user sent now cuts the stream, or the wait before a retry, short: what
/// streamed stays.
async fn stream(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    continues: bool,
) -> Result<Option<Trigger>, TurnError> {
    s.update(|core| core.set_stage(TurnStage::Streaming));
    let policy = s.config.retry;
    let mut attempt = 1;
    let mut refusal_compacted = false;
    loop {
        let request = request(s, cancel).await?;
        let request_id = request.request_id.clone();
        let start = s.read(|core| core.transcript.len());
        let model = s.read(|core| core.model.clone());
        let window = cancel.guard(compaction::window_in_use(s, &model)).await?;
        let mut runtime = s
            .update(|core| core.take_runtime())
            .expect("the provider runtime is in its slot between runs");
        let request_cancel = request.cancel.clone();
        let read = read(s, cancel, window, &model, runtime.run(request), &request_cancel).await;
        // However the request ended, no call of it is being written.
        s.update(|core| core.end_writing());
        s.return_runtime(runtime);
        let failure = match read? {
            Ok(reached) => {
                s.update(|core| core.set_stage(TurnStage::Preparing));
                compaction::report_context_usage(s, cancel).await;
                return Ok(reached);
            }
            Err(failure) => with_request_id(failure, &request_id),
        };
        // A scan that passes the whole window cuts before it.
        let unwindable = s.read(|core| match resume_point_within(core.transcript.blocks()) {
            Some(point) => core.transcript.start() + point.cut <= start,
            None => core.transcript.start() > 0,
        });
        // The same request would be refused again. Refused for its tokens:
        // one pass, then the request built from the compacted history.
        let too_long = failure.code == Some(ErrorCode::ContextLengthExceeded);
        if too_long && unwindable && !refusal_compacted && s.config.compaction.automatic() {
            refusal_compacted = true;
            drop_attempt(s, start).await?;
            let trigger = Trigger::TokenRefusal {
                request_id: request_id.clone(),
                message: failure.message.clone(),
            };
            if compact_before_request(s, cancel, continues, &trigger).await? {
                continue;
            }
        }
        if !(unwindable && policy.retries(attempt, &failure)) {
            let report = ErrorReport::from(&failure);
            s.update(|core| core.record_failure(&report));
            return Err(TurnError::Failed(Box::new(report)));
        }
        drop_attempt(s, start).await?;
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
                return Ok(None);
            }
            slept = cancel.guard(tokio::time::sleep(delay)) => slept?,
        }
        attempt += 1;
    }
}

/// One compaction pass, started by `trigger`, before a request; a pass that
/// compacted inside a running turn appends a `resume` block, while before a
/// turn's first request its new input is last already. Returns whether it
/// compacted.
async fn compact_before_request(
    s: &Rc<SessionShared>,
    cancel: &TurnCancel,
    continues: bool,
    trigger: &Trigger,
) -> Result<bool, TurnError> {
    let compacted = compaction::compacting(s, compaction::run_pass(s, cancel, trigger)).await?;
    if compacted && continues {
        s.update(|core| core.push_resume(None));
    }
    Ok(compacted)
}

/// Drops what a failed attempt wrote, cutting the history to its first
/// `cut` blocks. The row says Requesting again, so the node waits for the
/// model again, unless its wait runs on from before any output
/// (`rendering.md` § Waiting for the model).
async fn drop_attempt(s: &SessionShared, cut: usize) -> Result<(), TurnError> {
    let blocks = s.read(|core| core.transcript.blocks()[..cut - core.transcript.start()].to_vec());
    persist::commit_rewrite(s, blocks).await?;
    s.update(SessionCore::start_waiting);
    Ok(())
}

/// The next request: what the context sources tell the node first, saved at
/// once, then the system prompt, the tools and the replay.
async fn request(s: &SessionShared, cancel: &TurnCancel) -> Result<InferenceRequest, TurnError> {
    // Only the context the model receives counts as seen: a block before
    // the last compaction boundary is summarized away, so its source tells
    // the model again (`transcript.md` § Context).
    let (seen, turn) = s.read(|core| {
        let seen: Vec<(String, String)> = core
            .replayed()
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
    let system_prompt = system_prompt(s, &model, cancel).await?.text();
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
) -> Result<SystemPrompt, TurnError> {
    match cancel.guard(s.runtime.system_prompt(model)).await? {
        Ok(prompt) => Ok(prompt),
        Err(message) => {
            let report = ErrorReport::agent(AgentCode::PromptUnavailable, message);
            s.update(|core| core.record_failure(&report));
            Err(TurnError::Failed(Box::new(report)))
        }
    }
}

/// Applies a run's events as they arrive. A thinking start waits until
/// something follows it, so that an empty lifecycle leaves nothing behind;
/// open answer text completes when anything but more text follows it. The
/// run's failure is handed back unrecorded, for the retry to decide on; the
/// inner result is the trigger of a pass when the usage of a response that
/// requested a tool reached the compaction threshold of the run's `model`,
/// whose window in use is `window`. A message the user sent now ends it
/// where it is, cancelling the request through `request_cancel`, as a Stop
/// would, and what streamed stays.
async fn read(
    s: &SessionShared,
    cancel: &TurnCancel,
    window: Option<u64>,
    model: &ModelSelection,
    mut events: ProviderRun<'_>,
    request_cancel: &CancellationToken,
) -> Result<Result<Option<Trigger>, ProviderFailure>, TurnError> {
    let mut reached = None;
    // The frames of an answered response are held until the turn goes on;
    // more of the run after it is the turn going on, as when Claude Code
    // answers two messages of one run in turns of their own (`rendering.md`
    // § Order and delivery).
    let mut holding = false;
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
            // What follows a failure is the retry's to decide: what was
            // held for the turn's end shows now.
            ProviderEvent::Error(failure) => {
                s.release_events();
                return Ok(Err(failure));
            }
            // Entries write no block: open text stays open.
            ProviderEvent::Entries { of, entries } => {
                s.update(|core| core.keep_entries(of, entries));
            }
            event => {
                if std::mem::take(&mut holding) {
                    s.release_events();
                }
                // The response's end, without a call to run, ends the turn
                // unless the turn goes on: its text completes, and the
                // closing save follows before the phase goes idle. The
                // events wait for that end, or for the turn to go on
                // (`run_turn`), so a page never sees the finished text of a
                // running turn that has nothing more to run.
                let answered = matches!(event, ProviderEvent::Response(_))
                    && s.read(|core| core.transcript.pending_tool_calls().is_empty());
                if answered {
                    s.hold_events();
                    holding = true;
                }
                let completes_text = !matches!(
                    event,
                    ProviderEvent::TextDelta(_) | ProviderEvent::ThinkingSignature(_)
                );
                // Every response's usage is checked, so a misreport is
                // logged; only a call that requested a tool starts a pass:
                // one that ended the turn as answered is not continued, and
                // the pass before the next turn compacts (`compaction.md` §
                // When compaction runs).
                if let ProviderEvent::Response(Some(usage)) = &event
                    && let Some(trigger) = s.config.compaction.reached(window, model, usage)
                    && !answered
                {
                    reached = Some(trigger);
                }
                // The page says Requesting under text the model finished,
                // so its end starts the wait again within the run, until
                // something else shows (`rendering.md` § Waiting for the
                // model); the response's end is the run's, after which the
                // turn ends or its next request waits. One update, so an
                // event that shows, such as a whole call, ends that wait
                // before any page hears of it.
                let within_run = !matches!(event, ProviderEvent::Response(_));
                s.update(|core| {
                    if completes_text && core.complete_tail_text() && within_run {
                        core.start_waiting();
                    }
                    core.apply_event(event);
                });
            }
        }
    }
    drop(events);
    complete_text(s);
    Ok(Ok(reached))
}

/// Marks open answer text at the end complete, so a Fork may end at it
/// (`conversation-fork.md` § Eligibility).
fn complete_text(s: &SessionShared) {
    s.update(|core| {
        core.complete_tail_text();
    });
}

/// Runs the calls the last response requested, after saving them as
/// executing, in steps (`crates/agent-tools/src/lib.rs`): consecutive
/// calls of a tool that runs together start together, and each other call
/// is a step of its own. A step ends when each of its calls has returned;
/// each result is recorded as its call returns, in its call's place, and a
/// Stop keeps those of the calls that returned before it. Input that arrives
/// meanwhile, but a human steer, is written after each step, unless the turn
/// is about to compact. An action that fails before its calls ran, such as
/// when the save before dispatch fails, completes them as aborted with that
/// failure as the reason, as a stop does, so the next request replays no call
/// without a result. Returns whether the response requested any call.
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
                record_result(s, &call, ToolOutcome::not_run(), None).await;
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
        let ran = if demi_provider_common::offers(&tools, &step[0].tool_name) {
            let (model, request_limits) =
                s.read(|core| (core.model.clone(), core.request_limits()));
            let handles: Vec<CallCommand> = step.iter().map(|_| CallCommand::default()).collect();
            s.update(|core| {
                for (call, handle) in step.iter().zip(&handles) {
                    core.calls.insert(call.tool_use_id.clone(), handle.clone());
                }
            });
            // Where the step's calls start: a call's files are judged
            // against it once its result is stored, whatever directory the
            // node works in by then (`edit-tracking.md` § Within the working
            // directory).
            let started_in = s.runtime.directory().await;
            let turn = s.read(SessionCore::turn);
            let invocations = step
                .iter()
                .zip(&handles)
                .map(|(call, handle)| ToolInvocation {
                    tool_use_id: call.tool_use_id.clone(),
                    tool_name: call.tool_name.clone(),
                    turn: turn.clone(),
                    input: tool_input(&call.input),
                    model: model.clone(),
                    request_limits,
                    arrival: InputArrival::new(s.arrivals.subscribe(), joined),
                    command: handle.clone(),
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
                        record_result(s, &step[index], outcome, started_in.as_deref()).await;
                    }
                    Ok(None) => break Ok(()),
                    Err(stopped) => {
                        // The backend's shutdown leaves the calls' commands
                        // running, which the calls see as they are dropped
                        // (`crates/agent-session/src/core.rs`).
                        if cancel.reason() == Some(CancelReason::Shutdown) {
                            for handle in &handles {
                                handle.keep_running();
                            }
                        }
                        break Err(stopped);
                    }
                }
            }
        } else {
            for call in &step {
                let outcome = ToolOutcome::error(format!("Tool not found: {}", call.tool_name));
                record_result(s, call, outcome, None).await;
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

/// `view` with the lists the conversation shows of the shell call that
/// `at` finds in the transcript (`edit-tracking.md` § What the conversation
/// shows): its own files, its run's and its request's, within the working
/// directory, counted from the edit copies in the conversation's blob
/// namespace. The call's own files are judged against `started_in`, the
/// directory it started in; with none they keep the marks `view` has, as a
/// call listed again does. A call the transcript no longer holds, as after
/// an edit cut it, keeps `view` as it is.
async fn with_lists(
    s: &SessionShared,
    at: impl Fn(&SessionCore) -> Option<usize>,
    mut view: ShellToolView,
    started_in: Option<&str>,
) -> ShellToolView {
    // A request that began before the window lists its calls from its own
    // `user` block: the shell calls between it and the window are read from
    // the store, and nothing else of it.
    let reaches = s.read(|core| {
        let start = core.transcript.start();
        let index = at(core)?;
        let opened = core.transcript.blocks()[..index - start]
            .iter()
            .any(|block| matches!(block, Block::User(_)));
        Some((start > 0 && !opened).then_some(start))
    });
    let Some(reaches) = reaches else {
        return view;
    };
    let earlier = match reaches {
        Some(start) => match request_calls_before(s, start).await {
            Ok(earlier) => Some((start, earlier)),
            Err(error) => {
                // The lists are made of the window's calls alone: the call
                // still completes, and a later listing of the request reads
                // them again.
                tracing::warn!(error = &error as &dyn std::error::Error, "the request's earlier calls were not read");
                None
            }
        },
        None => None,
    };
    let planned = s.read(|core| {
        let start = core.transcript.start();
        let index = at(core)?;
        let window = core.transcript.blocks();
        Some(match &earlier {
            // The window still starts where the calls were read up to.
            Some((from, earlier)) if *from == start => {
                let blocks: Vec<Block> = earlier.iter().chain(window).cloned().collect();
                changes::plan(&blocks, index - start + earlier.len(), &view, started_in)
            }
            _ => changes::plan(window, index - start, &view, started_in),
        })
    });
    let Some(planned) = planned else {
        return view;
    };
    let counted = changes::count(planned.uncounted(), s.store.blobs()).await;
    planned.finish(&mut view, &counted);
    view
}

/// The `user` block that opened the request the window starts inside and
/// the tool calls after it before the window's `start`, from the store;
/// none when no request began before the window.
async fn request_calls_before(s: &SessionShared, start: usize) -> Result<Vec<Block>, StoreError> {
    let Some(opened) = s.store.last_of(Kind::User, start).await? else {
        return Ok(Vec::new());
    };
    let read = s.store.blocks_of(vec![Kind::User, Kind::ToolCall], opened..start).await?;
    Ok(read.into_iter().map(|(_, block)| block).collect())
}

/// Lists again the shell calls after `tool_use_id` in its request that
/// completed before it, as the calls of one step can: each call's lists
/// are of the calls before it in the transcript, whatever order they
/// returned in, and are final once the calls before it have returned.
async fn relist_later_calls(s: &SessionShared, tool_use_id: &str) {
    let later: Vec<(BlockId, ShellToolView)> = s.read(|core| {
        let blocks = core.transcript.blocks();
        let Some(index) = core.transcript.completed_call(tool_use_id) else {
            return Vec::new();
        };
        changes::later_in_request(blocks, index - core.transcript.start())
            .into_iter()
            .filter_map(|at| match &blocks[at] {
                Block::ToolCall(call) => match &call.view {
                    Some(ToolView::Shell(view)) => Some((call.id.clone(), view.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    });
    for (id, view) in later {
        let at = |core: &SessionCore| core.transcript.position(&id);
        // Its own files were judged as it was stored.
        let view = with_lists(s, at, view, None).await;
        s.update(|core| core.replace_view(&id, ToolView::Shell(view)));
    }
}

/// Records the result of `call`, and watches the command it left running;
/// its media are stored before it enters the transcript, and the session
/// holds their bytes (`transcript.md` § Media).
async fn record_result(
    s: &Rc<SessionShared>,
    call: &PendingCall,
    outcome: ToolOutcome,
    started_in: Option<&str>,
) {
    let (output, held) = media::store_result(outcome.output, s.store.blobs()).await;
    // A call that ran a command lists what later calls changed.
    let ran = matches!(outcome.view, Some(ToolView::Shell(_)));
    let view = match outcome.view {
        Some(ToolView::Shell(view)) => {
            let at = |core: &SessionCore| core.transcript.executing_call(&call.tool_use_id);
            Some(ToolView::Shell(with_lists(s, at, view, started_in).await))
        }
        view => view,
    };
    s.update(|core| {
        core.media.absorb(held);
        core.complete_tool_call(&call.tool_use_id, output, outcome.is_error, view);
    });
    if ran {
        relist_later_calls(s, &call.tool_use_id).await;
    }
    match outcome.effect {
        Some(ToolEffect::Background {
            command,
            interval_ms,
            ran,
        }) => {
            s.update(|core| core.watch_command(command.clone(), interval_ms));
            reports::start_watch(s, &command, ran);
        }
        None => {}
    }
}
