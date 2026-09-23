//! The OpenAI Responses stream as Demi reads it, shared by every provider
//! that speaks it (the OpenAI Responses API and the Codex backend, over
//! server-sent events or a WebSocket): the events Demi maps, decoded in two
//! steps by their `type`, and their mapping onto a run's events.
//!
//! The types describe only the fields Demi reads. A reasoning item keeps
//! every other field, because it is sent back to the vendor with the
//! transcript.

use std::{collections::HashMap, sync::LazyLock};

use bytes::Bytes;
use demi_core::{FailureSource, ProviderErrorDiagnostics, TokenUsage};
use futures_util::{Stream, StreamExt, future};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{
    ErrorCode, ProviderEvent, ProviderFailure, ToolCall, tagged_wire,
    wire::{
        ReportedString, Tagged, Vendor, WireError, decode_tagged, sse_data, tool_input,
        undecodable, usage_with_cached_input,
    },
};

tagged_wire! {
    /// The stream events Demi maps; any other type, such as
    /// `response.created` or one the vendor adds, is skipped.
    #[derive(Debug, Clone, PartialEq)]
    pub enum ResponsesEvent {
        "response.output_item.added" => ItemAdded(ItemAdded),
        "response.output_item.done" => ItemDone(ItemDone),
        "response.output_text.delta" => TextDelta(Delta),
        "response.reasoning_text.delta" => ReasoningTextDelta(Delta),
        "response.reasoning_summary_text.delta" => ReasoningSummaryDelta(Delta),
        "response.function_call_arguments.delta" => ArgumentsDelta(ArgumentsDelta),
        "response.function_call_arguments.done" => ArgumentsDone(ArgumentsDone),
        "response.completed" => Completed(Completed),
        "response.failed" => Failed(Failed),
        "response.incomplete" => Incomplete(Incomplete),
        "error" => Error(ErrorEvent),
    }
}

/// A decoded event with the text it arrived as: a failure keeps that text
/// as its record (`failures-and-recovery.md` § The failure record).
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub event: ResponsesEvent,
    pub text: String,
}

/// Decodes one frame's data: `None` for the `[DONE]` sentinel, which is not
/// JSON, and for an event type Demi does not map; an error for a mapped type
/// whose payload is malformed.
pub fn decode_frame(data: &str) -> Result<Option<Received>, WireError> {
    if data.trim() == "[DONE]" {
        return Ok(None);
    }
    let event = decode_tagged::<ResponsesEvent>(data)?;
    Ok(event.map(|event| Received {
        event,
        text: data.to_owned(),
    }))
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ItemAdded {
    #[serde(default)]
    pub item: Option<Tagged<ResponsesItem>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ItemDone {
    #[serde(default)]
    pub item: Option<Tagged<ResponsesItem>>,
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
}

/// One increment of the text the vendor streams.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Delta {
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ArgumentsDelta {
    #[serde(default)]
    pub item_id: Option<String>,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ArgumentsDone {
    #[serde(default)]
    pub item_id: Option<String>,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Completed {
    #[serde(default)]
    pub response: Option<CompletedResponse>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CompletedResponse {
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// Token counts as the API reports them: whole numbers, or absent. The input
/// count includes the cached prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub input_tokens_details: Option<InputTokensDetails>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct InputTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

impl Usage {
    /// The run's usage, with the cached prefix kept apart from the input.
    pub fn token_usage(&self) -> TokenUsage {
        let cached = self.input_tokens_details.and_then(|details| details.cached_tokens);
        usage_with_cached_input(self.input_tokens, self.output_tokens, cached)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Failed {
    #[serde(default)]
    pub response: Option<FailedResponse>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FailedResponse {
    #[serde(default)]
    pub id: ReportedString,
    #[serde(default)]
    pub error: Option<VendorError>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Incomplete {
    #[serde(default)]
    pub response: Option<IncompleteResponse>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IncompleteResponse {
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IncompleteDetails {
    #[serde(default)]
    pub reason: ReportedString,
}

/// A stream's `error` event. Server-sent events carry the failure flat as
/// `{message, code}`; the Codex WebSocket nests it under `error`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ErrorEvent {
    #[serde(default)]
    pub message: ReportedString,
    #[serde(default)]
    pub code: ReportedString,
    #[serde(default)]
    pub error: Option<VendorError>,
}

/// A vendor's error object, as failed responses and error events carry it.
/// Its fields are only reported, so anything but a string reads as absent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VendorError {
    #[serde(default)]
    pub code: ReportedString,
    #[serde(default, rename = "type")]
    pub kind: ReportedString,
    #[serde(default)]
    pub message: ReportedString,
    #[serde(default)]
    pub request_id: ReportedString,
    #[serde(default, rename = "requestId")]
    pub request_id_camel: ReportedString,
}

tagged_wire! {
    /// The output items Demi maps; any other, such as a web search call, is
    /// `None` inside [`Tagged`].
    #[derive(Debug, Clone, PartialEq)]
    pub enum ResponsesItem {
        "reasoning" => Reasoning(ReasoningItem),
        "message" => Message(MessageItem),
        "function_call" => FunctionCall(FunctionCallItem),
    }
}

/// A reasoning item. It travels back to the vendor with the transcript, so
/// it keeps the fields Demi does not read, and its `type` is part of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningItem {
    #[serde(rename = "type")]
    pub kind: ReasoningType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Vec<ReasoningPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ReasoningPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The one `type` of a reasoning item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningType {
    Reasoning,
}

/// One part of a reasoning item's summary or content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningPart {
    pub text: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ReasoningItem {
    /// The item's text: its summary, or its content when it has no summary.
    pub fn text(&self) -> String {
        let join = |parts: &Option<Vec<ReasoningPart>>| {
            parts
                .iter()
                .flatten()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let summary = join(&self.summary);
        if summary.is_empty() {
            join(&self.content)
        } else {
            summary
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MessageItem {
    #[serde(default)]
    pub content: Option<Vec<Tagged<MessagePart>>>,
}

impl MessageItem {
    /// The message's text: its output text parts and refusals.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .flatten()
            .filter_map(|part| match &part.0 {
                Some(MessagePart::OutputText(part)) => Some(part.text.as_str()),
                Some(MessagePart::Refusal(part)) => Some(part.refusal.as_str()),
                None => None,
            })
            .collect()
    }
}

tagged_wire! {
    /// The message content parts Demi maps.
    #[derive(Debug, Clone, PartialEq)]
    pub enum MessagePart {
        "output_text" => OutputText(OutputText),
        "refusal" => Refusal(Refusal),
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OutputText {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Refusal {
    pub refusal: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FunctionCallItem {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// The tool-use id of a function call: the vendor identifies a call by its
/// call id and its item id and needs both back, so Demi's one id carries the
/// pair as `<call_id>|<item_id>`.
pub fn tool_use_id(call_id: &str, item_id: &str) -> String {
    format!("{call_id}|{item_id}")
}

/// The call id and, when it has one, the item id of a tool-use id.
pub fn split_tool_use_id(tool_use_id: &str) -> (&str, Option<&str>) {
    match tool_use_id.split_once('|') {
        Some((call_id, item_id)) => (call_id, Some(item_id)),
        None => (tool_use_id, None),
    }
}

/// The events of a Responses body sent as server-sent events, each with the
/// text it arrived as, until the body ends or `cancel` fires. A body that
/// breaks off or a frame that cannot be decoded is the stream's last item,
/// as a failure.
pub fn sse_events<'a, S>(
    body: S,
    label: &'static str,
    cancel: CancellationToken,
) -> impl Stream<Item = Result<Received, ProviderFailure>> + 'a
where
    S: Stream<Item = reqwest::Result<Bytes>> + 'a,
{
    sse_data(body)
        .take_until(cancel.cancelled_owned())
        .filter_map(move |frame| {
            let decoded = match frame {
                Err(error) => Some(Err(ProviderFailure::event_stream(label, error))),
                Ok(data) => match decode_frame(&data) {
                    Ok(received) => received.map(Ok),
                    Err(error) => Some(Err(undecodable(label, &error, &data))),
                },
            };
            future::ready(decoded)
        })
}

/// Maps a Responses stream onto a run's events (`providers.md` § A run). The
/// run ends after the completion event, a failure or the stream's last
/// item; a stream that ends without its completion event still ends it with
/// a response, with zero usage. `events` ends when `cancel` fires, and then
/// the run ends without a further event. `signature_tag` is the prefix the
/// provider puts on the reasoning items it receives, so that it replays only
/// its own vendor's.
pub fn map_events<'a, S>(
    events: S,
    vendor: Vendor,
    signature_tag: &'static str,
    cancel: CancellationToken,
) -> impl Stream<Item = ProviderEvent> + 'a
where
    S: Stream<Item = Result<Received, ProviderFailure>> + 'a,
{
    async_stream::stream! {
        let mut events = std::pin::pin!(events);
        let mut mapper = Mapper::new(vendor, signature_tag);
        let mut out = Vec::new();
        while let Some(next) = events.next().await {
            let received = match next {
                Ok(received) => received,
                Err(failure) => {
                    yield ProviderEvent::Error(failure);
                    return;
                }
            };
            let ended = mapper.event(received, &mut out);
            for event in out.drain(..) {
                yield event;
            }
            if ended {
                return;
            }
        }
        if !cancel.is_cancelled() {
            yield ProviderEvent::Response(TokenUsage::default());
        }
    }
}

/// What one response is streaming: tool-call arguments arrive as deltas
/// keyed by item id, and a finished item's whole text is only emitted when
/// no delta carried it.
struct Mapper {
    vendor: Vendor,
    signature_tag: &'static str,
    /// The function call in flight, for argument deltas without an item id.
    current_call: Option<String>,
    /// Streamed arguments, by item id.
    arguments: HashMap<String, String>,
    reasoning_streamed: bool,
    text_streamed: bool,
}

impl Mapper {
    fn new(vendor: Vendor, signature_tag: &'static str) -> Self {
        Self {
            vendor,
            signature_tag,
            current_call: None,
            arguments: HashMap::new(),
            reasoning_streamed: false,
            text_streamed: false,
        }
    }

    /// Pushes the run's events for one received event onto `out`; `true`
    /// when the event ends the run.
    fn event(&mut self, received: Received, out: &mut Vec<ProviderEvent>) -> bool {
        match received.event {
            ResponsesEvent::ItemAdded(added) => {
                match added.item.and_then(|item| item.0) {
                    Some(ResponsesItem::Reasoning(_)) => out.push(ProviderEvent::ThinkingStart),
                    Some(ResponsesItem::FunctionCall(call)) => {
                        if let Some(id) = call.id {
                            self.arguments.insert(id.clone(), call.arguments.unwrap_or_default());
                            self.current_call = Some(id);
                        }
                    }
                    Some(ResponsesItem::Message(_)) | None => {}
                }
                false
            }
            ResponsesEvent::ReasoningSummaryDelta(delta) | ResponsesEvent::ReasoningTextDelta(delta) => {
                self.reasoning_streamed = true;
                if !delta.delta.is_empty() {
                    out.push(ProviderEvent::ThinkingDelta(delta.delta));
                }
                false
            }
            ResponsesEvent::TextDelta(delta) => {
                self.text_streamed = true;
                if !delta.delta.is_empty() {
                    out.push(ProviderEvent::TextDelta(delta.delta));
                }
                false
            }
            ResponsesEvent::ArgumentsDelta(delta) => {
                if let Some(id) = delta.item_id.or_else(|| self.current_call.clone()) {
                    self.arguments.entry(id).or_default().push_str(&delta.delta);
                }
                false
            }
            ResponsesEvent::ArgumentsDone(done) => {
                if let Some(id) = done.item_id.or_else(|| self.current_call.clone()) {
                    self.arguments.insert(id, done.arguments);
                }
                false
            }
            ResponsesEvent::ItemDone(done) => {
                self.item_done(done, out);
                false
            }
            ResponsesEvent::Completed(completed) => {
                let usage = completed.response.and_then(|response| response.usage);
                let usage = usage.map(|usage| usage.token_usage()).unwrap_or_default();
                out.push(ProviderEvent::Response(usage));
                true
            }
            ResponsesEvent::Failed(failed) => {
                let response = failed.response;
                let response_id = response.as_ref().and_then(|response| response.id.0.clone());
                let error = response.and_then(|response| response.error);
                let fallback = format!("{} response failed", self.vendor.label);
                let failure = self.failure(error.as_ref(), None, None, fallback, response_id, &received.text);
                out.push(ProviderEvent::Error(failure));
                true
            }
            ResponsesEvent::Incomplete(incomplete) => {
                let reason = incomplete
                    .response
                    .and_then(|response| response.incomplete_details)
                    .and_then(|details| details.reason.into_inner())
                    .unwrap_or_else(|| "unknown".to_owned());
                let code = if reason == "max_output_tokens" {
                    ErrorCode::ContextLengthExceeded
                } else {
                    ErrorCode::Incomplete
                };
                let message = format!(
                    "Incomplete {} response returned, reason: {reason}",
                    self.vendor.label
                );
                let failure = ProviderFailure {
                    message,
                    code: Some(code),
                    diagnostics: Some(stream_diagnostics(None, None, None, &received.text)),
                    retry_after: None,
                };
                out.push(ProviderEvent::Error(failure));
                true
            }
            ResponsesEvent::Error(event) => {
                let fallback = format!("{} stream error", self.vendor.label);
                let failure = self.failure(
                    event.error.as_ref(),
                    event.message.into_inner(),
                    event.code.into_inner(),
                    fallback,
                    None,
                    &received.text,
                );
                out.push(ProviderEvent::Error(failure));
                true
            }
        }
    }

    fn item_done(&mut self, done: ItemDone, out: &mut Vec<ProviderEvent>) {
        match done.item.and_then(|item| item.0) {
            Some(ResponsesItem::Reasoning(item)) => {
                // The finished text only when no delta carried it (a
                // response that did not stream), or it would arrive twice.
                if !self.reasoning_streamed {
                    let text = item.text();
                    if !text.is_empty() {
                        out.push(ProviderEvent::ThinkingDelta(text));
                    }
                }
                // A reasoning item is strings and JSON values, which always
                // serialize.
                let json = serde_json::to_string(&item).expect("a reasoning item serializes");
                out.push(ProviderEvent::ThinkingSignature(format!("{}{json}", self.signature_tag)));
                self.reasoning_streamed = false;
            }
            Some(ResponsesItem::Message(item)) => {
                if !self.text_streamed {
                    let text = item.text();
                    if !text.is_empty() {
                        out.push(ProviderEvent::TextDelta(text));
                    }
                }
                self.text_streamed = false;
            }
            Some(ResponsesItem::FunctionCall(call)) => {
                let item_id = call.id.or(done.item_id);
                let call_id = call.call_id.or(done.call_id);
                if let (Some(item_id), Some(call_id), Some(name)) = (&item_id, &call_id, call.name) {
                    let arguments = self
                        .arguments
                        .get(item_id)
                        .cloned()
                        .or(call.arguments)
                        .unwrap_or_else(|| "{}".to_owned());
                    out.push(ProviderEvent::ToolCall(ToolCall {
                        tool_use_id: tool_use_id(call_id, item_id),
                        tool_name: name,
                        input: tool_input(&arguments),
                    }));
                }
                if let Some(item_id) = item_id {
                    self.arguments.remove(&item_id);
                    if self.current_call.as_deref() == Some(item_id.as_str()) {
                        self.current_call = None;
                    }
                }
            }
            None => {}
        }
    }

    /// A failure the vendor reported in the stream: its message, else the
    /// event's own, else `fallback`; its code read from the vendor's code and
    /// message; the vendor's request and response ids; the frame as its
    /// record; and the wait the vendor's reader finds in it.
    fn failure(
        &self,
        error: Option<&VendorError>,
        message: Option<String>,
        code: Option<String>,
        fallback: String,
        response_id: Option<String>,
        text: &str,
    ) -> ProviderFailure {
        let message = message
            .or_else(|| error.and_then(|error| error.message.0.clone()))
            .unwrap_or(fallback);
        let code = code
            .or_else(|| error.and_then(|error| error.code.0.clone()))
            .or_else(|| error.and_then(|error| error.kind.0.clone()));
        let request_id = error
            .and_then(|error| error.request_id.0.clone().or_else(|| error.request_id_camel.0.clone()))
            .or_else(|| request_id_in(&message));
        let failure = ProviderFailure {
            code: ErrorCode::classify(code.as_deref(), &message),
            message,
            diagnostics: Some(stream_diagnostics(code, request_id, response_id, text)),
            retry_after: None,
        };
        failure.with_retry_wait(self.vendor.reader, self.vendor.clock.now())
    }
}

fn stream_diagnostics(
    provider_code: Option<String>,
    provider_request_id: Option<String>,
    provider_response_id: Option<String>,
    text: &str,
) -> ProviderErrorDiagnostics {
    ProviderErrorDiagnostics {
        source: FailureSource::Stream,
        client_request_id: None,
        provider_request_id,
        provider_response_id,
        provider_code,
        http_status: None,
        upstream: Some(text.to_owned()),
    }
}

/// The request id OpenAI embeds in a failure's message, which it asks users
/// to quote.
fn request_id_in(message: &str) -> Option<String> {
    static REQUEST_ID: LazyLock<Regex> = LazyLock::new(|| {
        // A constant of this module; a test reads a message through it.
        Regex::new(r"(?i)request ID ([A-Za-z0-9-]+)").expect("the request id pattern compiles")
    });
    REQUEST_ID
        .captures(message)
        .map(|captures| captures[1].to_owned())
}
