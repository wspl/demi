//! A session's runtime and its runs (`claude-code.md` § Requests over
//! stream-json, § Tool-call batches, § Process lifetime). The runtime keeps
//! at most one CLI process. A run takes it, or starts a new one, and hands it
//! back only where the conversation can go on in it: at the turn's `result`,
//! and at a batch of tool calls whose results the next run delivers. It hands
//! it back before the run's last event, since the agent stops reading a run
//! at its failure. Every other end closes it, or drops it, which kills it.

use std::rc::Rc;
use std::sync::Arc;

use async_stream::stream;
use demi_provider_common::{
    InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun, ProviderRuntime, RequestLimits, ToolResultKinds,
    Secret,
};
use demi_shared_types::Model;
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use futures_util::stream::{self, Stream};

use crate::Shared;
use crate::live::{LiveCli, Next};
use crate::output::{ContentBlock, Line, Printed, StreamEvent, TurnEnd};
use crate::placement::Placement;

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
        }
    }
}

impl ProviderRuntime for ClaudeCodeRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let span = tracing::info_span!(
            "claude_code_run",
            provider = %self.shared.id,
            session = %request.session_id,
            request = %request.request_id,
        );
        let run = run(self, request);
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
) -> impl Stream<Item = ProviderEvent> + '_ {
    stream! {
        let cancel = request.cancel.clone();
        // The token the vendor refused in the first attempt.
        let mut refused: Option<Secret> = None;
        'attempt: loop {
            // A second attempt starts afresh: the first closed its process.
            let kept = runtime.live.take();
            let mut live = match kept {
                Some(mut live) if live.serves(&request) => {
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
                            for event in entries {
                                yield event;
                            }
                            live
                        }
                    }
                }
                previous => {
                    if let Some(previous) = previous {
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
                    Next::End => {
                        if let Some(failure) = live.end().await {
                            yield ProviderEvent::Error(failure);
                        }
                        return;
                    }
                    // A call that opens before the model's message streamed is
                    // a batch of one; once the process streams, calls come with
                    // the batch its message_stop ends.
                    Next::Opened(call) => {
                        if live.streamed {
                            continue;
                        }
                        live.collecting.clear();
                        live.held = vec![call.clone()];
                        let call = ProviderEvent::ToolCall(call);
                        live.mirror.observe(&call);
                        let entries = live.mirror.place(&request, false);
                        runtime.live = Some(live);
                        yield call;
                        for event in entries {
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
                        let id = message.message_id();
                        for printed in message.content() {
                            live.mirror.printed(id.clone(), printed.raw, false);
                        }
                    }
                    Some(Line::Assistant(message)) => {
                        // Once the process streams, a whole message repeats what
                        // streamed, but its tool uses are whole only here.
                        let streamed = live.streamed;
                        let id = message.message_id();
                        for Printed { raw, block } in message.content() {
                            match block {
                                Some(ContentBlock::ToolUse(tool_use)) => match tool_use.call() {
                                    Ok(call) => live.collect(call),
                                    Err(error) => {
                                        yield ProviderEvent::Error(ProviderFailure::protocol(error.to_string(), text));
                                        live.close().await;
                                        return;
                                    }
                                },
                                Some(block) if !streamed => {
                                    let events = block.events();
                                    let wrote = !events.is_empty();
                                    for event in events {
                                        live.mirror.observe(&event);
                                        yielded = true;
                                        yield event;
                                    }
                                    live.mirror.printed(id.clone(), raw, wrote);
                                }
                                // What streamed wrote the block: reasoning
                                // opens one at its start, and text with its
                                // first piece; redacted reasoning streams no
                                // event.
                                Some(block) => {
                                    let wrote = match &block {
                                        ContentBlock::Thinking(_) => true,
                                        ContentBlock::Text(text) => !text.is_empty(),
                                        ContentBlock::RedactedThinking(_) | ContentBlock::ToolUse(_) => false,
                                    };
                                    live.mirror.printed(id.clone(), raw, wrote);
                                }
                                None => live.mirror.printed(id.clone(), raw, false),
                            }
                        }
                    }
                    Some(Line::TranscriptMirror(line)) => {
                        for event in live.mirror.mirrored(line.entries, &request) {
                            yield event;
                        }
                    }
                    Some(Line::StreamEvent(line)) => {
                        live.streamed = true;
                        let Some(event) = line.event() else {
                            continue;
                        };
                        if let StreamEvent::MessageStop(_) = event {
                            live.writing.clear();
                            if live.collecting.is_empty() {
                                continue;
                            }
                            if live.mcp.is_none() {
                                let message = "Claude Code called a tool, but the request offered none";
                                yield ProviderEvent::Error(ProviderFailure::protocol(message, text));
                                live.close().await;
                                return;
                            }
                            live.held = std::mem::take(&mut live.collecting);
                            let batch: Vec<ProviderEvent> =
                                live.held.iter().cloned().map(ProviderEvent::ToolCall).collect();
                            for call in &batch {
                                live.mirror.observe(call);
                            }
                            let entries = live.mirror.place(&request, false);
                            runtime.live = Some(live);
                            for event in batch.into_iter().chain(entries) {
                                yield event;
                            }
                            return;
                        }
                        for event in event.events(&mut live.writing) {
                            live.mirror.observe(&event);
                            yielded = true;
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
                            TurnEnd::Answered(usage) => yield ProviderEvent::Response(usage),
                            TurnEnd::Failed { message, code, status } => {
                                let mut failure = ProviderFailure {
                                    code,
                                    ..ProviderFailure::protocol(message, text)
                                };
                                if let Some(diagnostics) = &mut failure.diagnostics {
                                    diagnostics.http_status = status;
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
