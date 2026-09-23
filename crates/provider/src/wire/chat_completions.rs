//! The OpenAI Chat Completions stream as Demi reads it, shared by every
//! provider that speaks it (OpenAI-compatible endpoints and the Grok Build
//! proxy), and its mapping onto a run's events. A chunk carries no `type`
//! tag, so there is nothing to skip: a malformed chunk is a protocol failure
//! that names the field.

use std::collections::BTreeMap;

use bytes::Bytes;
use demi_core::{FailureSource, ProviderErrorDiagnostics, TokenUsage};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::{
    ErrorCode, ProviderEvent, ProviderFailure, ToolCall,
    wire::{
        ReportedString, Vendor, WireError, decode_untagged, sse_data, tool_input, undecodable,
        usage_with_cached_input,
    },
};

/// One streamed chunk. With `stream_options.include_usage`, only the last
/// chunk carries counts; the ones before spell their absence as `null`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Chunk {
    #[serde(default)]
    pub choices: Option<Vec<Choice>>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub error: Option<ChunkError>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub delta: Option<ChoiceDelta>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// A choice's increment. Vendors spell "nothing in this chunk" as `null`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChoiceDelta {
    #[serde(default)]
    pub content: Option<String>,
    /// Reasoning that compatible vendors, such as DeepSeek and Grok Build,
    /// stream beside the answer.
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// One increment of a tool call. `index` groups the increments of one call;
/// the id and the name arrive once, the arguments in pieces.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ToolCallDelta {
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDelta>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// Token counts as the API reports them; the prompt count includes the
/// cached prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

impl Usage {
    /// The run's usage, with the cached prefix kept apart from the input.
    pub fn token_usage(&self) -> TokenUsage {
        let cached = self.prompt_tokens_details.and_then(|details| details.cached_tokens);
        usage_with_cached_input(self.prompt_tokens, self.completion_tokens, cached)
    }
}

/// A failure the vendor reports inside the stream. Its fields are only
/// reported, so anything but a string reads as absent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChunkError {
    #[serde(default)]
    pub message: ReportedString,
    #[serde(default)]
    pub code: ReportedString,
    #[serde(default, rename = "type")]
    pub kind: ReportedString,
}

/// Decodes one chunk; the error names the offending field's path.
pub fn decode_chunk(data: &str) -> Result<Chunk, WireError> {
    decode_untagged(data)
}

/// Maps a Chat Completions body sent as server-sent events onto a run's
/// events (`providers.md` § A run), until the body ends or `cancel` fires.
/// The run ends with one response, whether the vendor closed the stream with
/// its `[DONE]` sentinel or just ended the body, after the tool calls
/// collected so far; a vendor error or an undecodable chunk ends it as a
/// failure instead. A cancelled run ends without a further event.
pub fn map_sse<'a, S>(
    body: S,
    vendor: Vendor,
    cancel: CancellationToken,
) -> impl Stream<Item = ProviderEvent> + 'a
where
    S: Stream<Item = reqwest::Result<Bytes>> + 'a,
{
    async_stream::stream! {
        let mut frames = std::pin::pin!(sse_data(body).take_until(cancel.clone().cancelled_owned()));
        let mut mapper = Mapper::default();
        let mut out = Vec::new();
        while let Some(frame) = frames.next().await {
            let data = match frame {
                Ok(data) => data,
                Err(error) => {
                    yield ProviderEvent::Error(ProviderFailure::event_stream(vendor.label, error));
                    return;
                }
            };
            let ended = mapper.frame(&data, &vendor, &mut out);
            for event in out.drain(..) {
                yield event;
            }
            if ended {
                return;
            }
        }
        if cancel.is_cancelled() {
            return;
        }
        mapper.finish(&mut out);
        for event in out.drain(..) {
            yield event;
        }
    }
}

/// A tool call assembled from the increments of one index.
#[derive(Debug, Default)]
struct Collected {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct Mapper {
    /// The calls being assembled, by index, so they flush in index order.
    calls: BTreeMap<u32, Collected>,
    thinking_started: bool,
    usage: TokenUsage,
}

impl Mapper {
    /// Pushes the run's events for one frame onto `out`; `true` when the
    /// frame ends the run.
    fn frame(&mut self, data: &str, vendor: &Vendor, out: &mut Vec<ProviderEvent>) -> bool {
        if data.trim() == "[DONE]" {
            self.finish(out);
            return true;
        }
        let chunk = match decode_chunk(data) {
            Ok(chunk) => chunk,
            Err(error) => {
                out.push(ProviderEvent::Error(undecodable(vendor.label, &error, data)));
                return true;
            }
        };
        if let Some(error) = chunk.error {
            out.push(ProviderEvent::Error(failure(error, vendor, data)));
            return true;
        }
        if let Some(usage) = chunk.usage {
            self.usage = usage.token_usage();
        }
        for choice in chunk.choices.into_iter().flatten() {
            if let Some(delta) = choice.delta {
                if let Some(reasoning) = delta.reasoning_content.filter(|text| !text.is_empty()) {
                    if !self.thinking_started {
                        self.thinking_started = true;
                        out.push(ProviderEvent::ThinkingStart);
                    }
                    out.push(ProviderEvent::ThinkingDelta(reasoning));
                }
                if let Some(content) = delta.content.filter(|text| !text.is_empty()) {
                    out.push(ProviderEvent::TextDelta(content));
                }
                for call in delta.tool_calls.into_iter().flatten() {
                    self.collect(call);
                }
            }
            if choice.finish_reason.as_deref() == Some("tool_calls") {
                self.flush(out);
            }
        }
        false
    }

    /// Folds one increment into the call it belongs to. A vendor that omits
    /// `index` sends one call at a time, in order.
    fn collect(&mut self, delta: ToolCallDelta) {
        let index = delta
            .index
            .unwrap_or_else(|| u32::try_from(self.calls.len()).unwrap_or(u32::MAX));
        let call = self.calls.entry(index).or_default();
        if let Some(id) = delta.id.filter(|id| !id.is_empty()) {
            call.id = id;
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name.filter(|name| !name.is_empty()) {
                call.name = name;
            }
            if let Some(arguments) = function.arguments {
                call.arguments.push_str(&arguments);
            }
        }
    }

    /// Emits the assembled calls in index order and forgets them. A call
    /// that never named its tool is dropped; one without an id gets
    /// `tool_call_<index>`.
    fn flush(&mut self, out: &mut Vec<ProviderEvent>) {
        for (index, call) in std::mem::take(&mut self.calls) {
            if call.name.is_empty() {
                continue;
            }
            let tool_use_id = if call.id.is_empty() {
                format!("tool_call_{index}")
            } else {
                call.id
            };
            let arguments = if call.arguments.is_empty() {
                "{}"
            } else {
                call.arguments.as_str()
            };
            out.push(ProviderEvent::ToolCall(ToolCall {
                tool_use_id,
                tool_name: call.name,
                input: tool_input(arguments),
            }));
        }
    }

    /// The end of the stream: the calls collected so far, then the response.
    fn finish(&mut self, out: &mut Vec<ProviderEvent>) {
        self.flush(out);
        out.push(ProviderEvent::Response(self.usage));
    }
}

/// A failure the vendor reported in a chunk: its message, else one naming
/// the vendor; its code read from the vendor's code and message; the chunk
/// as its record; and the wait the vendor's reader finds in it.
fn failure(error: ChunkError, vendor: &Vendor, data: &str) -> ProviderFailure {
    let message = error
        .message
        .into_inner()
        .unwrap_or_else(|| format!("{} stream error", vendor.label));
    let code = error.code.into_inner().or_else(|| error.kind.into_inner());
    let failure = ProviderFailure {
        code: ErrorCode::classify(code.as_deref(), &message),
        message,
        diagnostics: Some(ProviderErrorDiagnostics {
            source: FailureSource::Stream,
            client_request_id: None,
            provider_request_id: None,
            provider_response_id: None,
            provider_code: code,
            http_status: None,
            upstream: Some(data.to_owned()),
        }),
        retry_after: None,
    };
    failure.with_retry_wait(vendor.reader, vendor.clock.now())
}
