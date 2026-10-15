//! A session's runtime and its runs (`claude-code.md` § Requests over
//! stream-json, § Tool-call batches, § Process lifetime). The runtime keeps
//! at most one CLI process. A run takes it, or starts a new one, and hands it
//! back only where the conversation can go on in it: at the turn's `result`,
//! and at a batch of tool calls whose results the next run delivers. It hands
//! it back before the run's last event, since the agent stops reading a run
//! at its failure. Every other end closes it, or drops it, which kills it.
//! When the next request is known but for its input, the runtime prepares
//! its process ahead, and the request takes that one as a kept process
//! when it is the request it was prepared for (`claude-code.md` §
//! Preparing a process ahead).

use std::rc::Rc;
use std::sync::Arc;

use async_stream::stream;
use demi_provider_common::{
    Ahead, InferenceItem, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun,
    ProviderRuntime, RequestLimits, Secret, ToolResultKinds, offers,
};
use demi_shared_types::Model;
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use futures_util::stream::{self, Stream};

use crate::Shared;
use crate::failure::read_failure;
use crate::live::{LiveCli, Next};
use crate::output::{Answered, ContentBlock, Line, Printed, StreamEvent, Streaming, TurnEnd};
use crate::placement::Placement;
use crate::prepared::Prepared;
use crate::timing::{self, RunTiming, Took};

/// The HTTP status of a call the vendor refused for its token.
const UNAUTHORIZED: u16 = 401;

/// A session's runtime.
pub(crate) struct ClaudeCodeRuntime {
    shared: Arc<Shared>,
    /// The client of the shard the runtime runs on.
    http: reqwest::Client,
    placement: Rc<dyn Placement>,
    /// The process kept from the last run.
    live: Option<LiveCli>,
    /// The process prepared for the next request; a runtime that keeps a
    /// process prepares none.
    prepared: Option<Prepared>,
}

impl ClaudeCodeRuntime {
    pub(crate) fn new(
        shared: Arc<Shared>,
        http: reqwest::Client,
        placement: Rc<dyn Placement>,
    ) -> Self {
        Self {
            shared,
            http,
            placement,
            live: None,
            prepared: None,
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
        Box::new(Self::new(
            self.shared.clone(),
            self.http.clone(),
            self.placement.clone(),
        ))
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {
            if let Some(live) = self.live.take() {
                live.close().await;
            }
            if let Some(prepared) = self.prepared.take() {
                prepared.close().await;
            }
        })
    }

    fn prepares_ahead(&self) -> bool {
        true
    }

    /// Prepares the process of the next request, `request` but for its
    /// input, when the runtime holds none that would serve it: as a page
    /// opens the conversation or the user picks another model, effort or
    /// account; and, as an action ends, when the kept process's token is
    /// about to expire. A prepared process the history moved away from is
    /// closed. A history that does not end with an answer prepares nothing.
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
                (Ahead::Opened | Ahead::Chosen, Some(live)) => {
                    !(live.continues(&request) && live.fresh())
                }
                (Ahead::Ended, Some(live)) => live.continues(&request) && !live.fresh(),
                (Ahead::Ended, None) => false,
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
            self.prepared = Some(Prepared::start(
                &self.shared,
                &self.http,
                &self.placement,
                request,
            ));
        })
    }

    /// The Anthropic API's, which the CLI sends its history to.
    fn request_limits(&self, model: &Model) -> RequestLimits {
        // Images as MCP image content; the CLI saves an MCP PDF resource to
        // a file of its own instead of giving it to the model (`providers.md`
        // § Media in tool results).
        RequestLimits {
            tool_results: ToolResultKinds::IMAGES,
            ..RequestLimits::anthropic_messages(model)
        }
    }
}

/// One run: the process kept for the session when it can go on with the
/// request, else a new one that resumes the session written from the
/// request's blocks; then the CLI's output until the turn's end or a batch
/// of tool calls, with the entries the CLI mirrors of its session. A turn the vendor
/// refuses for its token is run once more in a new process with a refreshed
/// token (`claude-code.md` § Accounts and sign-in).
fn run(
    runtime: &mut ClaudeCodeRuntime,
    request: InferenceRequest,
    mut timing: RunTiming,
) -> impl Stream<Item = ProviderEvent> + '_ {
    stream! {
        let cancel = request.cancel.clone();
        // The token the vendor refused in the first attempt.
        let mut refused: Option<Secret> = None;
        'attempt: loop {
            // A second attempt starts afresh: the first closed its process.
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
            let mut live = match kept {
                Some((mut live, took)) if live.serves(&request) => {
                    timing.took(took, live.started.take());
                    if let Some(mcp) = &live.mcp {
                        mcp.offer(request.tools.clone());
                    }
                    let continued = async {
                        let mut entries = live.mirror.begin(&request);
                        entries.extend(live.skip_leftovers(&request).await?);
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
                    let started = LiveCli::start(
                        &runtime.shared,
                        &runtime.http,
                        &*runtime.placement,
                        &request,
                        refused.as_ref(),
                    );
                    let mut live = match cancel.run_until_cancelled(started).await {
                        // Giving the start up drops what it started, which kills
                        // it.
                        None => return,
                        Some(Err(failure)) => {
                            yield ProviderEvent::Error(failure);
                            return;
                        }
                        Some(Ok(live)) => live,
                    };
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
            // Whether the run has yielded anything of this attempt, which a
            // second attempt would then repeat.
            let mut yielded = false;
            // The line the CLI made up for a call the vendor refused, as
            // received, which the failure's record keeps beside the result:
            // only it carries the refusal's reset.
            let mut refusal: Option<String> = None;
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
                    // A notice of a refused call repeats the failure the result
                    // reports, so the failure shows once; it writes no block.
                    Some(Line::Assistant(message)) if message.is_refusal_notice() => {
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
                            complete = Some(live.abandon_streaming());
                            live.last_call.answered(message.usage());
                        }
                        for Printed { raw, block } in message.content() {
                            match block {
                                Some(ContentBlock::ToolUse(tool_use)) => match tool_use.call() {
                                    // A call of a tool the request does not
                                    // offer is the CLI's to answer, which it
                                    // does with an error, and the model goes
                                    // on in this run (`claude-code.md` §
                                    // Tool-call batches).
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
                                            yielded = true;
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
                        live.last_call.observe(&event);
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
                            yielded = true;
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
                        // The vendor refused the process's token, before
                        // anything of the attempt reached the agent: the
                        // token is refreshed and the request sent again,
                        // once, in a new process, which resumes the
                        // transcript.
                        if let TurnEnd::Failed {
                            status: Some(UNAUTHORIZED),
                            ..
                        } = &end
                            && refused.is_none()
                            && !yielded
                        {
                            refused = Some(live.token.clone());
                            live.close().await;
                            continue 'attempt;
                        }
                        runtime.live = Some(live);
                        match end {
                            // Without a call's own count the response reports
                            // none: the result's usage sums the turn's calls,
                            // which no request measured.
                            TurnEnd::Answered => yield ProviderEvent::response(usage, crate::FAMILY, &request.model_id),
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
