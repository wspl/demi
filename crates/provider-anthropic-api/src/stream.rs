//! One run: the request, then the Messages API's event stream mapped onto
//! provider events (`providers.md` § A run).

use std::{collections::HashMap, sync::Arc};

use demi_core::{Clock, FailureSource, ProviderErrorDiagnostics, TokenUsage};
use demi_provider::{
    ErrorCode, InferenceRequest, ProviderEvent, ProviderFailure, ToolCall, encode_body,
    http_failure, read_http_failure, tagged_wire,
    wire::{NonEmpty, ReportedString, Tagged, decode_tagged, sse_data},
};
use futures_util::{Stream, StreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;

use crate::{
    Shared,
    request::{self, SIGNATURE_TAG},
};

/// How failure messages name the vendor.
const LABEL: &str = "Anthropic";

/// The Messages API version the provider speaks.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// A run of `request`. It checks its token before each step and races every
/// wait on the vendor against it; a cancelled run ends without a further
/// event, and dropping the stream drops the connection.
pub(crate) fn run(
    shared: Arc<Shared>,
    http: reqwest::Client,
    request: InferenceRequest,
) -> impl Stream<Item = ProviderEvent> {
    async_stream::stream! {
        let cancel = request.cancel.clone();
        if cancel.is_cancelled() {
            return;
        }
        // A body with images is megabytes of base64, built off the shard.
        let body = match encode_body(LABEL, move || request::encode(&request)).await {
            Ok(body) => body,
            Err(failure) => {
                yield ProviderEvent::Error(failure);
                return;
            }
        };
        let send = http
            .post(shared.messages_url.clone())
            .header("x-api-key", shared.api_key.clone())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .body(body)
            .send();
        let sent = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            sent = send => Some(sent),
        };
        let Some(sent) = sent else {
            return;
        };
        let response = match sent {
            Ok(response) => response,
            Err(error) => {
                yield ProviderEvent::Error(ProviderFailure::transport(LABEL, error));
                return;
            }
        };
        if !response.status().is_success() {
            let failure = tokio::select! {
                biased;
                () = cancel.cancelled() => None,
                failure = http_failure(response, LABEL, read_http_failure, &*shared.clock) => Some(failure),
            };
            if let Some(failure) = failure {
                yield ProviderEvent::Error(failure);
            }
            return;
        }
        let mut frames = Box::pin(sse_data(response.bytes_stream()));
        let mut mapper = Mapper::new(shared.clock.clone());
        loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => None,
                next = frames.next() => Some(next),
            };
            let Some(next) = next else {
                return;
            };
            match next {
                // A stream that ends without `message_stop` still ends the
                // run with its usage.
                None => {
                    yield ProviderEvent::Response(mapper.usage);
                    return;
                }
                Some(Err(error)) => {
                    yield ProviderEvent::Error(ProviderFailure::event_stream(LABEL, error));
                    return;
                }
                Some(Ok(data)) => match mapper.frame(&data) {
                    Next::Nothing => {}
                    Next::Event(event) => yield event,
                    Next::Last(event) => {
                        yield event;
                        return;
                    }
                },
            }
        }
    }
}

/// What one frame means for the run.
enum Next {
    Nothing,
    Event(ProviderEvent),
    /// The run's last event.
    Last(ProviderEvent),
}

/// The state of one stream: the tool-use blocks being streamed, by index,
/// and the usage so far.
struct Mapper {
    tools: HashMap<u32, ToolBlock>,
    usage: TokenUsage,
    clock: Arc<dyn Clock>,
}

/// A tool-use block whose input arrives as JSON text, one piece per delta.
struct ToolBlock {
    id: String,
    name: String,
    initial_input: Option<serde_json::Value>,
    input_json: String,
}

impl Mapper {
    fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            tools: HashMap::new(),
            usage: TokenUsage::default(),
            clock,
        }
    }

    fn frame(&mut self, data: &str) -> Next {
        let event = match decode_tagged::<StreamEvent>(data) {
            Ok(Some(event)) => event,
            // An event type Demi does not map, such as `ping`.
            Ok(None) => return Next::Nothing,
            Err(error) => {
                let message = format!("{LABEL} API stream sent a frame Demi cannot read: {error}");
                return Next::Last(ProviderEvent::Error(ProviderFailure::protocol(message, data)));
            }
        };
        match event {
            StreamEvent::Error(event) => Next::Last(ProviderEvent::Error(self.failure(event, data))),
            StreamEvent::MessageStart(start) => {
                merge(&mut self.usage, start.message.usage.as_ref());
                Next::Nothing
            }
            StreamEvent::MessageDelta(delta) => {
                merge(&mut self.usage, delta.usage.as_ref());
                Next::Nothing
            }
            StreamEvent::MessageStop(_) => Next::Last(ProviderEvent::Response(self.usage)),
            StreamEvent::BlockStart(start) => self.block_start(start),
            StreamEvent::BlockDelta(delta) => self.block_delta(delta),
            StreamEvent::BlockStop(stop) => match self.tools.remove(&stop.index) {
                Some(block) => Next::Event(ProviderEvent::ToolCall(block.call())),
                None => Next::Nothing,
            },
        }
    }

    fn block_start(&mut self, start: BlockStart) -> Next {
        let Tagged(Some(block)) = start.content_block else {
            return Next::Nothing;
        };
        match block {
            ContentBlock::ToolUse(tool) => {
                self.tools.insert(
                    start.index,
                    ToolBlock {
                        id: tool.id.0,
                        name: tool.name.0,
                        initial_input: tool.input,
                        input_json: String::new(),
                    },
                );
                Next::Nothing
            }
            ContentBlock::Thinking(_) => Next::Event(ProviderEvent::ThinkingStart),
            ContentBlock::RedactedThinking(block) => {
                Next::Event(ProviderEvent::RedactedThinking(tagged(&block.data)))
            }
            // A text block usually opens empty and fills through deltas.
            ContentBlock::Text(block) if block.text.is_empty() => Next::Nothing,
            ContentBlock::Text(block) => Next::Event(ProviderEvent::TextDelta(block.text)),
        }
    }

    fn block_delta(&mut self, delta: BlockDelta) -> Next {
        let Tagged(Some(piece)) = delta.delta else {
            return Next::Nothing;
        };
        match piece {
            Delta::Text(piece) if piece.text.is_empty() => Next::Nothing,
            Delta::Text(piece) => Next::Event(ProviderEvent::TextDelta(piece.text)),
            Delta::Thinking(piece) if piece.thinking.is_empty() => Next::Nothing,
            Delta::Thinking(piece) => Next::Event(ProviderEvent::ThinkingDelta(piece.thinking)),
            Delta::Signature(piece) if piece.signature.is_empty() => Next::Nothing,
            Delta::Signature(piece) => {
                Next::Event(ProviderEvent::ThinkingSignature(tagged(&piece.signature)))
            }
            Delta::InputJson(piece) => {
                if let Some(block) = self.tools.get_mut(&delta.index) {
                    block.input_json.push_str(&piece.partial_json);
                }
                Next::Nothing
            }
        }
    }

    /// An error event: the failure rides in a nested `error` whose `type` is
    /// the vendor's error class; without it only the event's own tag is left
    /// to classify by. The record is the frame as received.
    fn failure(&self, event: ErrorEvent, data: &str) -> ProviderFailure {
        let vendor_code = event.error.as_ref().and_then(|error| error.kind.as_deref());
        let message = event
            .error
            .as_ref()
            .and_then(|error| error.message.as_deref())
            .or(event.message.as_deref())
            .unwrap_or("Anthropic API stream error")
            .to_owned();
        let failure = ProviderFailure {
            code: ErrorCode::classify(Some(vendor_code.unwrap_or("error")), &message),
            message,
            diagnostics: Some(ProviderErrorDiagnostics {
                source: FailureSource::Stream,
                client_request_id: None,
                provider_request_id: None,
                provider_response_id: None,
                provider_code: vendor_code.map(str::to_owned),
                http_status: None,
                upstream: Some(data.to_owned()),
            }),
            retry_after: None,
        };
        failure.with_retry_wait(read_http_failure, self.clock.now())
    }
}

impl ToolBlock {
    /// The call, with the input its deltas streamed, else the input its start
    /// carried, else an empty object. Streamed text that is not valid JSON is
    /// kept as the string the vendor sent (`runtime.md` § Replay).
    fn call(self) -> ToolCall {
        let input = if self.input_json.is_empty() {
            self.initial_input
                .unwrap_or_else(|| serde_json::Value::Object(Default::default()))
        } else {
            match serde_json::from_str(&self.input_json) {
                Ok(input) => input,
                Err(_) => serde_json::Value::String(self.input_json),
            }
        };
        ToolCall {
            tool_use_id: self.id,
            tool_name: self.name,
            input,
        }
    }
}

/// A signature or redacted data as the transcript keeps it.
fn tagged(value: &str) -> String {
    format!("{SIGNATURE_TAG}{value}")
}

/// Folds one event's counts into the total. A stream reports each count
/// once, the input side on `message_start` and the output side on
/// `message_delta`, and repeats the others as zero or omits them, so a count
/// that is zero or absent keeps the total.
fn merge(total: &mut TokenUsage, reported: Option<&Usage>) {
    let Some(reported) = reported else {
        return;
    };
    let keep = |current: u64, count: Option<u64>| count.filter(|count| *count > 0).unwrap_or(current);
    total.input_tokens = keep(total.input_tokens, reported.input_tokens);
    total.output_tokens = keep(total.output_tokens, reported.output_tokens);
    total.cache_read_tokens = keep(total.cache_read_tokens, reported.cache_read_input_tokens);
    total.cache_write_tokens = keep(total.cache_write_tokens, reported.cache_creation_input_tokens);
}

tagged_wire! {
    /// The stream events Demi maps; any other type, such as `ping` or one the
    /// vendor adds, is skipped.
    enum StreamEvent {
        "message_start" => MessageStart(MessageStart),
        "content_block_start" => BlockStart(BlockStart),
        "content_block_delta" => BlockDelta(BlockDelta),
        "content_block_stop" => BlockStop(BlockStop),
        "message_delta" => MessageDelta(MessageDelta),
        "message_stop" => MessageStop(MessageStop),
        "error" => Error(ErrorEvent),
    }
}

#[derive(Deserialize)]
struct MessageStart {
    message: StartedMessage,
}

#[derive(Deserialize)]
struct StartedMessage {
    usage: Option<Usage>,
}

/// Token counts as the API reports them: whole numbers, or absent.
#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

/// `index` ties a block's start, deltas and stop together, so a block event
/// without one is a protocol failure rather than a write to block 0.
#[derive(Deserialize)]
struct BlockStart {
    index: u32,
    content_block: Tagged<ContentBlock>,
}

#[derive(Deserialize)]
struct BlockDelta {
    index: u32,
    delta: Tagged<Delta>,
}

#[derive(Deserialize)]
struct BlockStop {
    index: u32,
}

#[derive(Deserialize)]
struct MessageDelta {
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct MessageStop {}

#[derive(Deserialize)]
struct ErrorEvent {
    #[serde(default)]
    message: ReportedString,
    error: Option<ErrorBody>,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default, rename = "type")]
    kind: ReportedString,
    #[serde(default)]
    message: ReportedString,
}

tagged_wire! {
    /// The content blocks Demi maps.
    enum ContentBlock {
        "text" => Text(TextBlock),
        "thinking" => Thinking(ThinkingBlock),
        "redacted_thinking" => RedactedThinking(RedactedThinkingBlock),
        "tool_use" => ToolUse(ToolUseBlock),
    }
}

#[derive(Deserialize)]
struct TextBlock {
    text: String,
}

#[derive(Deserialize)]
struct ThinkingBlock {}

#[derive(Deserialize)]
struct RedactedThinkingBlock {
    data: String,
}

#[derive(Deserialize)]
struct ToolUseBlock {
    id: NonEmpty,
    name: NonEmpty,
    input: Option<serde_json::Value>,
}

tagged_wire! {
    /// The block deltas Demi maps.
    enum Delta {
        "text_delta" => Text(TextDelta),
        "thinking_delta" => Thinking(ThinkingDelta),
        "signature_delta" => Signature(SignatureDelta),
        "input_json_delta" => InputJson(InputJsonDelta),
    }
}

#[derive(Deserialize)]
struct TextDelta {
    text: String,
}

#[derive(Deserialize)]
struct ThinkingDelta {
    thinking: String,
}

#[derive(Deserialize)]
struct SignatureDelta {
    signature: String,
}

#[derive(Deserialize)]
struct InputJsonDelta {
    partial_json: String,
}

