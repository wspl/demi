//! A session's runtime and its runs (`claude-code.md` § Requests over
//! stream-json, § Tool-call batches, § Process lifetime). The runtime keeps
//! at most one CLI process. A run takes it, or starts a new one, and hands it
//! back only where the conversation can go on in it: after the turn's
//! `result`, and after a batch of tool calls whose results the next run
//! delivers. Every other end closes it, or drops it, which kills it.

use std::rc::Rc;
use std::sync::Arc;

use async_stream::stream;
use demi_provider::{
    InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun, ProviderRuntime,
};
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use futures_util::stream::{self, Stream};

use crate::Shared;
use crate::live::{LiveCli, Next};
use crate::output::{ContentBlock, Line, StreamEvent, TurnEnd};
use crate::placement::Placement;

/// A session's runtime.
pub(crate) struct ClaudeCodeRuntime {
    shared: Arc<Shared>,
    placement: Rc<dyn Placement>,
    /// The process kept from the last run.
    live: Option<LiveCli>,
}

impl ClaudeCodeRuntime {
    pub(crate) fn new(shared: Arc<Shared>, placement: Rc<dyn Placement>) -> Self {
        Self {
            shared,
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
        Box::new(Self::new(self.shared.clone(), self.placement.clone()))
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {
            if let Some(live) = self.live.take() {
                live.close().await;
            }
        })
    }
}

/// One run: the process kept for the session when it can go on with the
/// request, else a new one that replays the transcript; then the CLI's
/// output until the turn's end or a batch of tool calls.
fn run(
    runtime: &mut ClaudeCodeRuntime,
    request: InferenceRequest,
) -> impl Stream<Item = ProviderEvent> + '_ {
    stream! {
        let cancel = request.cancel.clone();
        let kept = runtime.live.take();
        let mut live = match kept {
            Some(mut live) if live.serves(&request) => {
                if let Some(mcp) = &live.mcp {
                    mcp.offer(request.tools.clone());
                }
                let continued = async {
                    live.skip_leftovers().await?;
                    if !live.held.is_empty() {
                        live.deliver(&request).await?;
                    }
                    live.send_new_user_messages(&request).await
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
                    Some(Ok(())) => live,
                }
            }
            previous => {
                if let Some(previous) = previous {
                    previous.close().await;
                }
                let started = LiveCli::start(&runtime.shared, &*runtime.placement, &request);
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
                    Some(Ok(())) => live,
                }
            }
        };
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
                    yield ProviderEvent::ToolCall(call);
                    runtime.live = Some(live);
                    return;
                }
                Next::Line(text, line) => (text, line),
            };
            match line {
                None | Some(Line::ControlResponse(_)) => {}
                Some(Line::Assistant(message)) => {
                    // Once the process streams, a whole message repeats what
                    // streamed, but its tool uses are whole only here.
                    let streamed = live.streamed;
                    for block in message.content() {
                        match block {
                            ContentBlock::ToolUse(tool_use) => match tool_use.call() {
                                Ok(call) => live.collect(call),
                                Err(error) => {
                                    yield ProviderEvent::Error(ProviderFailure::protocol(error.to_string(), text));
                                    live.close().await;
                                    return;
                                }
                            },
                            block if !streamed => {
                                for event in block.events() {
                                    yield event;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some(Line::StreamEvent(line)) => {
                    live.streamed = true;
                    let Some(event) = line.event() else {
                        continue;
                    };
                    if let StreamEvent::MessageStop(_) = event {
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
                        for call in live.held.clone() {
                            yield ProviderEvent::ToolCall(call);
                        }
                        runtime.live = Some(live);
                        return;
                    }
                    for event in event.events() {
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
                    match line.end() {
                        TurnEnd::Answered(usage) => yield ProviderEvent::Response(usage),
                        TurnEnd::Failed { message, code } => {
                            yield ProviderEvent::Error(ProviderFailure {
                                code,
                                ..ProviderFailure::protocol(message, text)
                            });
                        }
                    }
                    runtime.live = Some(live);
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
