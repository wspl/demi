//! A session's runtime and its runs (`crates/provider-claude-code/src/mcp.rs`,
//! `crates/provider-claude-code/src/live.rs`). The runtime keeps
//! at most one CLI process. A run takes it, or starts a new one, and hands it
//! back only where the conversation can go on in it: at the `result` printed
//! once the CLI took every message the run wrote, and at a batch of tool calls
//! whose results the next
//! run delivers. It hands
//! it back before the run's last event, since the agent stops reading a run
//! at its failure. Every other end closes it, SIGTERM first.
//! When the next request is known but for its input, the runtime prepares
//! its process ahead, and the request takes that one as a kept process
//! when it is the request it was prepared for (`crates/provider-claude-code/src/prepared.rs`).
//!
//! A request that gains nothing over what a kept process received, no new user
//! message and no results for held calls, is a retry of a turn that failed: the
//! process already holds the transcript and would wait for input that never
//! comes, so a new one resumes the history the retry cut back to.
//!
//! The CLI answers user messages that reach it at different times in turns of
//! their own (two subagents' receipts, a steer written while it waits for a
//! batch's results), each with a `result`. Ending the run at the first would
//! leave the second answer to reach no request: the next run would take part of
//! it for its own, and the answer to its own input would print after it ended,
//! never shown. So the CLI prints each user message back as it takes it into a
//! turn (`--replay-user-messages`), and a `result` ends the run only once every
//! message the run wrote has been printed back before it. A message the CLI
//! answers itself without the model, such as `/cost`, is never printed back;
//! its `result` stands for the oldest message not yet printed back, so a
//! message starting with `/` never leaves a run waiting.

use std::rc::Rc;
use std::sync::Arc;

use async_stream::stream;
use demi_provider_common::credentials::{AuthFailure, AuthReason};
use demi_provider_common::{
    Ahead, CallEvent, CallOutcome, InferenceItem, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun,
    ProviderRuntime, RequestLimits, ToolResultKinds, offers, quota::Observation,
};
use demi_shared_types::Model;
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use futures_util::stream::{self, Stream};

use crate::Shared;
use crate::failure::read_failure;
use crate::live::{LiveCli, Next};
use crate::output::{Answered, ContentBlock, Line, Printed, StreamEvent, Streaming, TurnEnd};
use crate::placement::{Placement, WorkLease};
use crate::prepared::Prepared;
use crate::timing::{self, RunTiming, Took};

/// The HTTP status of a call the vendor refused for its sign-in.
const UNAUTHORIZED: u16 = 401;

/// A session's runtime.
pub(crate) struct ClaudeCodeRuntime {
    shared: Arc<Shared>,
    placement: Rc<dyn Placement>,
    /// The process kept from the last run.
    live: Option<LiveCli>,
    /// The process prepared for the next request; a runtime that keeps a
    /// process prepares none.
    prepared: Option<Prepared>,
    /// The hold of the machine for the session's action that infers here,
    /// from its first request to its end, across the tool calls between its
    /// requests, so a CLI that waits on a long tool keeps its machine awake
    /// (`managed-hosts.md` § The Claude Code machine).
    action: Option<WorkLease>,
}

impl ClaudeCodeRuntime {
    pub(crate) fn new(shared: Arc<Shared>, placement: Rc<dyn Placement>) -> Self {
        Self {
            shared,
            placement,
            live: None,
            prepared: None,
            action: None,
        }
    }
}

/// Whether `items` end with the model's answer, so a process resuming them
/// waits for input: they are no empty history, which the CLI cannot
/// resume, and end in no input still to answer, such as a message whose
/// turn failed or a call's result.
fn answered(items: &[InferenceItem]) -> bool {
    matches!(
        items.last(),
        Some(
            InferenceItem::AssistantText { .. }
                | InferenceItem::AssistantThinking { .. }
                | InferenceItem::AssistantRedactedThinking { .. }
        )
    )
}

impl ProviderRuntime for ClaudeCodeRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let span = timing::run_span(&self.shared.id, &request.session_id, &request.request_id);
        let timing = RunTiming::new(span.clone());
        let run = run(self, request, timing);
        let mut run = Box::pin(run);
        stream::poll_fn(move |context| {
            let _entered = span.enter();
            run.as_mut().poll_next(context)
        })
        .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self::new(self.shared.clone(), self.placement.clone()))
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {
            self.action = None;
            if let Some(live) = self.live.take() {
                live.close().await;
            }
            if let Some(prepared) = self.prepared.take() {
                prepared.close().await;
            }
        })
    }

    /// The action ended, completed, failed or stopped: the machine is no
    /// longer held for it.
    fn idle(&mut self) {
        self.action = None;
    }

    fn prepares_ahead(&self) -> bool {
        true
    }

    /// Prepares the process of the next request, `request` but for its
    /// input, when the runtime holds none that would serve it: as a page
    /// opens the conversation or the user picks another model, effort or
    /// account. A prepared process the history moved away from is closed,
    /// as an action's end finds it. A history that does not end with an answer prepares nothing.
    /// Nothing here waits for a process: the session readies while the
    /// user may be sending, so what it closes ends by itself.
    fn prepare(&mut self, request: InferenceRequest, ahead: Ahead) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some(prepared) = self.prepared.take() {
                if prepared.prepared_for(&request) {
                    self.prepared = Some(prepared);
                    return;
                }
                tokio::task::spawn_local(prepared.close());
            }
            let wanted = match (ahead, &self.live) {
                (Ahead::Opened | Ahead::Chosen, None) => true,
                (Ahead::Opened | Ahead::Chosen, Some(live)) => !live.continues(&request),
                (Ahead::Ended, _) => false,
            };
            if !wanted || !answered(&request.items) {
                return;
            }
            if let Some(live) = self.live.take() {
                tokio::task::spawn_local(live.close());
            }
            tracing::debug!(
                provider = %self.shared.id,
                session = %request.session_id,
                ?ahead,
                "preparing a Claude Code process ahead"
            );
            self.prepared = Some(Prepared::start(&self.shared, &self.placement, request));
        })
    }

    /// The Anthropic API's, which the CLI sends its history to.
    fn request_limits(&self, model: &Model) -> RequestLimits {
        // Images as MCP image content; the CLI saves an MCP PDF resource to
        // a file of its own instead of giving it to the model
        // (`crates/provider-common/src/contract.rs`).
        RequestLimits {
            tool_results: ToolResultKinds::IMAGES,
            ..RequestLimits::anthropic_messages(model)
        }
    }
}

/// One run: the process kept for the session when it can go on with the
/// request, else a new one that resumes the session written from the
/// request's blocks; then the CLI's output until the end of the turn that
/// answers the last message the run wrote, each earlier turn's answer a
/// response of its own, or a batch of tool calls, with the entries the CLI
/// mirrors of its session. A turn
/// whose sign-in the vendor refused fails with the authentication error and
/// marks the account signed out (`crates/provider-claude-code/src/account.rs`).
fn run(
    runtime: &mut ClaudeCodeRuntime,
    request: InferenceRequest,
    mut timing: RunTiming,
) -> impl Stream<Item = ProviderEvent> + '_ {
    stream! {
        let cancel = request.cancel.clone();
        // The run's first call starts with the run, as the usage ledger
        // counts it.
        let run_started = runtime.shared.clock.now();
        // The machine stays awake from the action's first request to its
        // end, however long its process waits on a tool's result.
        if runtime.action.is_none() {
            runtime.action = Some(runtime.placement.work());
        }
        {
            let mut kept = runtime.live.take().map(|live| (live, Took::Kept));
            if let Some(prepared) = runtime.prepared.take() {
                if prepared.serves(&request) {
                    match cancel.run_until_cancelled(prepared.take()).await {
                        // The task closes a process nobody waits for.
                        None => return,
                        Some(Some(live)) => kept = Some((live, Took::Prepared)),
                        // Its preparation failed: the request starts its
                        // process as it would have.
                        Some(None) => {}
                    }
                } else {
                    prepared.close().await;
                }
            }
            // A process kept at a `result` prints nothing until it is given
            // input; one that did went on without a request, and a new one
            // starts from the transcript. One kept at a batch goes on with its turn, which
            // this run reads.
            let kept = match kept {
                Some((mut live, Took::Kept)) if live.held.is_empty() => {
                    if live.quiet().await {
                        Some((live, Took::Kept))
                    } else {
                        live.close().await;
                        None
                    }
                }
                kept => kept,
            };
            let mut live = match kept {
                Some((mut live, took)) if live.serves(&request) => {
                    timing.took(took, live.started.take());
                    live.run_started(run_started);
                    if let Some(mcp) = &live.mcp {
                        mcp.offer(request.tools.clone());
                    }
                    let continued = async {
                        let entries = live.mirror.begin(&request);
                        if !live.held.is_empty() {
                            live.deliver(&request).await?;
                        }
                        live.send_new_user_messages(&request).await?;
                        Ok(entries)
                    };
                    match cancel.run_until_cancelled(continued).await {
                        None => {
                            live.close().await;
                            return;
                        }
                        Some(Err(failure)) => {
                            yield ProviderEvent::Error(failure);
                            live.close().await;
                            return;
                        }
                        Some(Ok(entries)) => {
                            timing.ready();
                            for event in entries {
                                yield event;
                            }
                            live
                        }
                    }
                }
                previous => {
                    if let Some((previous, _)) = previous {
                        previous.close().await;
                    }
                    let started = LiveCli::start(&runtime.shared, &*runtime.placement, &request);
                    let mut live = match cancel.run_until_cancelled(started).await {
                        // Giving the start up drops what it started, which
                        // closes it.
                        None => return,
                        Some(Err(failure)) => {
                            yield ProviderEvent::Error(failure);
                            return;
                        }
                        Some(Ok(live)) => live,
                    };
                    live.run_started(run_started);
                    match cancel.run_until_cancelled(live.prepare(&request)).await {
                        None => {
                            live.close().await;
                            return;
                        }
                        Some(Err(failure)) => {
                            yield ProviderEvent::Error(failure);
                            live.close().await;
                            return;
                        }
                        Some(Ok(())) => {
                            timing.took(Took::New, live.started.take());
                            timing.ready();
                            // A new process mirrored nothing yet.
                            live.mirror.begin(&request);
                            live
                        }
                    }
                }
            };
            // The line the CLI made up for a call the vendor refused, as
            // received, which the failure's record keeps beside the result:
            // only it carries the refusal's reset.
            let mut refusal: Option<String> = None;
            // The vendor refused the account's sign-in: the CLI said
            // `authentication_failed` on the line it made up for the call.
            let mut signed_out = false;
            loop {
                let (text, line) = match live.next(&cancel).await {
                    Next::Cancelled => {
                        live.close().await;
                        return;
                    }
                    Next::Broken(failure) => {
                        yield ProviderEvent::Error(failure);
                        live.close().await;
                        return;
                    }
                    // The output ended before the `result` line that says the
                    // reply is complete: the process's own failure, else a
                    // reply cut short.
                    Next::End => {
                        let failure = live.end().await.unwrap_or_else(ProviderFailure::cut_short);
                        yield ProviderEvent::Error(failure);
                        return;
                    }
                    // A call that opens while no response streams, before
                    // the model's message was read, is a batch of one; a
                    // call of a streaming response comes with the batch its
                    // message_stop ends.
                    Next::Opened(call) => {
                        if live.streaming.is_some() {
                            continue;
                        }
                        let events = live.hand_over(vec![call], &request);
                        runtime.live = Some(live);
                        for event in events {
                            yield event;
                        }
                        return;
                    }
                    Next::Line(text, line) => (text, line),
                };
                match line {
                    None | Some(Line::ControlResponse(_)) => {}
                    // A message the run wrote, taken into a turn; the
                    // results of calls are printed too, and count for
                    // nothing.
                    Some(Line::User(line)) => {
                        if line.is_replay {
                            live.unreplayed = live.unreplayed.saturating_sub(1);
                            live.turn_began = true;
                        }
                    }
                    // The quota the vendor reported is the call's open now,
                    // or the one that ended last; the snapshot took it as
                    // the line was read.
                    Some(Line::RateLimit(_)) => {
                        let windows = serde_json::from_str(&text)
                            .ok()
                            .and_then(|value| runtime.shared.quota.windows(Observation::CliLine(&value)))
                            .filter(|windows| !windows.is_empty());
                        if let Some(windows) = windows {
                            yield ProviderEvent::Call(CallEvent::Observed(windows));
                        }
                    }
                    // A notice of a refused call repeats the failure the result
                    // reports, so the failure shows once; it writes no block.
                    Some(Line::Assistant(message)) if message.is_refusal_notice() => {
                        // The refused call is one of its own, which the
                        // run's failure ends.
                        yield live.call_started();
                        signed_out |= message.refused_sign_in();
                        refusal = Some(text);
                        let id = message.message_id();
                        for printed in message.content() {
                            live.mirror.printed(id.clone(), printed.raw, false);
                        }
                    }
                    Some(Line::Assistant(message)) => {
                        // A whole line of the response being streamed repeats
                        // a block that streamed, but its tool uses are whole
                        // only here. Any other line is a complete answer, as
                        // the CLI's retry without streaming after a stream
                        // failed: it replaces the abandoned response, whose
                        // calls never run, shows without what of it already
                        // showed, and hands its batch on at once, since no
                        // message_stop follows it.
                        let id = message.message_id();
                        // What a complete answer replaces; none for a line
                        // that repeats what streamed.
                        let mut complete = None;
                        if !live.streaming.as_ref().is_some_and(|streaming| streaming.repeats(&id)) {
                            // The abandoned stream's call failed; the
                            // complete answer is a call of its own.
                            if live.streaming.is_some() {
                                yield live.last_call.ended(CallOutcome::Refused(None));
                            }
                            complete = Some(live.abandon_streaming());
                            yield live.call_started();
                            live.last_call.answered(id.clone(), message.usage());
                            yield live.last_call.ended(CallOutcome::Answered);
                        }
                        for Printed { raw, block } in message.content() {
                            match block {
                                Some(ContentBlock::ToolUse(tool_use)) => match tool_use.call() {
                                    // A call of a tool the request does not
                                    // offer is the CLI's to answer, which it
                                    // does with an error, and the model goes
                                    // on in this run (`crates/provider-claude-code/src/mcp.rs`).
                                    Ok(call) if !offers(&request.tools, &call.tool_name) => {}
                                    Ok(call) => live.collect(call),
                                    Err(error) => {
                                        yield ProviderEvent::Error(ProviderFailure::protocol(error.to_string(), text));
                                        live.close().await;
                                        return;
                                    }
                                },
                                Some(block) => match &mut complete {
                                    Some(replaced) => {
                                        let (events, answered) = replaced.events(block);
                                        for event in events {
                                            live.mirror.observe(&event);
                                            timing.delta();
                                            yield event;
                                        }
                                        match answered {
                                            Answered::Repeats(index) => live.mirror.printed_at(id.clone(), raw, index),
                                            Answered::New(wrote) => live.mirror.printed(id.clone(), raw, wrote),
                                        }
                                    }
                                    // What streamed wrote the block: reasoning
                                    // opens one at its start, and text with its
                                    // first piece; redacted reasoning streams no
                                    // event.
                                    None => {
                                        let wrote = match &block {
                                            ContentBlock::Thinking(_) => true,
                                            ContentBlock::Text(text) => !text.is_empty(),
                                            ContentBlock::RedactedThinking(_) | ContentBlock::ToolUse(_) => false,
                                        };
                                        live.mirror.printed(id.clone(), raw, wrote);
                                    }
                                },
                                None => live.mirror.printed(id.clone(), raw, false),
                            }
                        }
                        if let Some(replaced) = complete.take() {
                            for event in live.mirror.left_out(replaced.unrepeated()) {
                                yield event;
                            }
                            if !live.collecting.is_empty() {
                                let batch = std::mem::take(&mut live.collecting);
                                let events = live.hand_over(batch, &request);
                                runtime.live = Some(live);
                                for event in events {
                                    yield event;
                                }
                                return;
                            }
                        }
                    }
                    Some(Line::TranscriptMirror(line)) => {
                        let streaming = live.streaming.as_ref().and_then(Streaming::id).map(str::to_owned);
                        for event in live.mirror.mirrored(line.entries, &request, streaming.as_deref()) {
                            yield event;
                        }
                    }
                    Some(Line::StreamEvent(line)) => {
                        let Some(event) = line.event() else {
                            continue;
                        };
                        // Each call is a usage ledger row: a message starts
                        // one, and its message_stop ends it; a message that
                        // starts while another streams abandons that call.
                        if let StreamEvent::MessageStart(_) = &event {
                            live.turn_began = true;
                            if live.streaming.is_some() {
                                yield live.last_call.ended(CallOutcome::Refused(None));
                            }
                            yield live.call_started();
                        }
                        live.last_call.observe(&event);
                        if let StreamEvent::MessageStop(_) = &event {
                            yield live.last_call.ended(CallOutcome::Answered);
                        }
                        match &event {
                            // A new message abandons one whose stream never
                            // stopped, and the calls it collected.
                            StreamEvent::MessageStart(start) => {
                                let abandoned = live.abandon_streaming();
                                for event in live.mirror.left_out(abandoned.unrepeated()) {
                                    yield event;
                                }
                                live.streaming = Some(Streaming::new(start.message_id()));
                            }
                            // The message's entries, held while it streamed,
                            // now have their blocks.
                            StreamEvent::MessageStop(_) => {
                                if live.collecting.is_empty() {
                                    live.streaming = None;
                                    live.writing.clear();
                                    for event in live.mirror.place(&request, false, None) {
                                        yield event;
                                    }
                                    continue;
                                }
                                let batch = std::mem::take(&mut live.collecting);
                                let events = live.hand_over(batch, &request);
                                runtime.live = Some(live);
                                for event in events {
                                    yield event;
                                }
                                return;
                            }
                            _ => {
                                live.streaming.get_or_insert_default();
                            }
                        }
                        for event in event.events(&mut live.writing) {
                            // Nor is such a call shown as being written.
                            if let ProviderEvent::ToolCallStart { tool_use_id, tool_name } = &event
                                && !offers(&request.tools, tool_name)
                            {
                                live.writing.retain(|_, writing| writing != tool_use_id);
                                continue;
                            }
                            live.mirror.observe(&event);
                            if let Some(streaming) = &mut live.streaming {
                                streaming.observe(&event, live.mirror.last_block());
                            }
                            timing.delta();
                            yield event;
                        }
                    }
                    Some(Line::ControlRequest(line)) => {
                        if let Err(failure) = live.control_request(line).await {
                            yield ProviderEvent::Error(failure);
                            live.close().await;
                            return;
                        }
                    }
                    Some(Line::Result(line)) => {
                        live.collecting.clear();
                        live.writing.clear();
                        live.streaming = None;
                        let usage = live.last_call.take();
                        let end = line.end();
                        // A turn the CLI answered itself, with no message
                        // printed back and no model call, as for a `/cost`
                        // it does not offer, took the oldest message not yet
                        // printed back. A failed turn is never one: a call the
                        // vendor refused starts no stream either.
                        if !std::mem::take(&mut live.turn_began) && matches!(end, TurnEnd::Answered) {
                            live.unreplayed = live.unreplayed.saturating_sub(1);
                        }
                        // The CLI has not taken every message the run wrote:
                        // this result ends a turn of an earlier one, and the
                        // run reads on to the turns of the rest.
                        let earlier = live.unreplayed > 0;
                        // The vendor refused the account's sign-in, which the
                        // CLI could not renew and emptied: the account waits
                        // for the user to sign in again, and rotation passes
                        // over it.
                        if let TurnEnd::Failed { status, .. } = &end
                            && (signed_out || *status == Some(UNAUTHORIZED))
                        {
                            live.close().await;
                            runtime.shared.auth.signed_out().await;
                            let record = match refusal {
                                Some(refusal) => format!("{refusal}\n{text}"),
                                None => text,
                            };
                            let failure = AuthFailure::new(crate::FAMILY, AuthReason::SignedOut).failure();
                            yield ProviderEvent::Error(ProviderFailure {
                                code: failure.code,
                                ..ProviderFailure::protocol(failure.message, record)
                            });
                            return;
                        }
                        match end {
                            // Without a call's own count the response reports
                            // none: the result's usage sums the turn's calls,
                            // which no request measured.
                            TurnEnd::Answered if earlier => {
                                refusal = None;
                                signed_out = false;
                                yield ProviderEvent::response(usage, crate::FAMILY, &request.model_id);
                                continue;
                            }
                            TurnEnd::Answered => {
                                runtime.live = Some(live);
                                yield ProviderEvent::response(usage, crate::FAMILY, &request.model_id);
                            }
                            TurnEnd::Failed { message, code, status } => {
                                let record = match refusal {
                                    Some(refusal) => format!("{refusal}\n{text}"),
                                    None => text,
                                };
                                let mut failure = ProviderFailure {
                                    code,
                                    ..ProviderFailure::protocol(message, record)
                                };
                                if let Some(diagnostics) = &mut failure.diagnostics {
                                    diagnostics.http_status = status;
                                }
                                // A usage limit waits until its window
                                // resets, longer than any retry waits.
                                let failure = failure.with_retry_wait(
                                    read_failure,
                                    runtime.shared.clock.now(),
                                );
                                // A process that would still answer a message
                                // of the run's after its failure is out of step
                                // with the requests: the next starts a new one.
                                if earlier {
                                    live.close().await;
                                } else {
                                    runtime.live = Some(live);
                                }
                                yield ProviderEvent::Error(failure);
                            }
                        }
                        return;
                    }
                    Some(Line::Error(line)) => {
                        let (message, code) = line.failure();
                        yield ProviderEvent::Error(ProviderFailure {
                            code,
                            ..ProviderFailure::protocol(message, text)
                        });
                        live.close().await;
                        return;
                    }
                }
            }
        }
    }
}
