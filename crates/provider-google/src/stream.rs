//! One run: the request, then Gemini's event stream mapped onto provider
//! events (`providers.md` § A run).

use std::sync::Arc;

use demi_core::{FailureSource, ProviderErrorDiagnostics, TokenUsage};
use demi_provider::{
    ErrorCode, InferenceRequest, ProviderEvent, ProviderFailure, ToolCall, encode_body,
    http_failure, read_http_failure,
    wire::{NonEmpty, ReportedString, decode_untagged, sse_data, undecodable},
};
use futures_util::{Stream, StreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;

use crate::{SIGNATURE_TAG, Shared, request};

/// How failure messages name the vendor.
const LABEL: &str = "Google";

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
        let url = shared.stream_url(&request.model_id);
        let body = encode_body(LABEL, move || request::encode(&request));
        let body = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            body = body => Some(body),
        };
        let body = match body {
            None => return,
            Some(Ok(body)) => body,
            Some(Err(failure)) => {
                yield ProviderEvent::Error(failure);
                return;
            }
        };
        let send = http
            .post(url)
            .header("x-goog-api-key", shared.api_key.clone())
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .body(body)
            .send();
        let sent = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            sent = send => Some(sent),
        };
        let response = match sent {
            None => return,
            Some(Ok(response)) => response,
            Some(Err(error)) => {
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
        let mut frames = std::pin::pin!(sse_data(response.bytes_stream()).take_until(cancel.clone().cancelled_owned()));
        let mut mapper = Mapper::new(shared);
        let mut out = Vec::new();
        while let Some(frame) = frames.next().await {
            let data = match frame {
                Ok(data) => data,
                Err(error) => {
                    yield ProviderEvent::Error(ProviderFailure::event_stream(LABEL, error));
                    return;
                }
            };
            let ended = mapper.frame(&data, &mut out);
            for event in out.drain(..) {
                yield event;
            }
            if ended {
                return;
            }
        }
        if !cancel.is_cancelled() {
            yield ProviderEvent::Response(mapper.usage);
        }
    }
}

/// The state of one stream: whether a thinking block is open, and the usage
/// so far.
struct Mapper {
    shared: Arc<Shared>,
    thinking_open: bool,
    usage: TokenUsage,
}

impl Mapper {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            thinking_open: false,
            usage: TokenUsage::default(),
        }
    }

    /// Pushes the run's events for one chunk onto `out`; `true` when the
    /// chunk ends the run.
    fn frame(&mut self, data: &str, out: &mut Vec<ProviderEvent>) -> bool {
        let chunk: Chunk = match decode_untagged(data) {
            Ok(chunk) => chunk,
            Err(error) => {
                out.push(ProviderEvent::Error(undecodable(LABEL, &error, data)));
                return true;
            }
        };
        if let Some(error) = chunk.error {
            out.push(ProviderEvent::Error(self.failure(error, data)));
            return true;
        }
        if let Some(usage) = chunk.usage_metadata {
            self.usage = usage.token_usage();
        }
        let parts = chunk
            .candidates
            .into_iter()
            .flatten()
            .filter_map(|candidate| candidate.content)
            .flat_map(|content| content.parts.into_iter().flatten());
        for part in parts {
            self.part(part, out);
        }
        false
    }

    fn part(&mut self, part: ResponsePart, out: &mut Vec<ProviderEvent>) {
        if let Some(call) = part.function_call {
            // The signature rides on a thinking item just before the call,
            // so it survives in the transcript in front of the call it
            // belongs to (`request.rs`).
            if let Some(signature) = part.thought_signature {
                if !self.thinking_open {
                    out.push(ProviderEvent::ThinkingStart);
                }
                out.push(ProviderEvent::ThinkingSignature(format!(
                    "{SIGNATURE_TAG}{signature}"
                )));
            }
            // A call without an id gets a unique one, so it never repeats an
            // id already in the transcript.
            let name = call.name.0;
            let tool_use_id = call
                .id
                .unwrap_or_else(|| format!("{name}_{}", uuid::Uuid::new_v4()));
            out.push(ProviderEvent::ToolCall(ToolCall {
                tool_use_id,
                tool_name: name,
                input: call.args.unwrap_or_else(|| serde_json::json!({})),
            }));
            self.thinking_open = false;
            return;
        }
        if part.thought == Some(true) {
            if !self.thinking_open {
                out.push(ProviderEvent::ThinkingStart);
                self.thinking_open = true;
            }
            if let Some(text) = part.text.filter(|text| !text.is_empty()) {
                out.push(ProviderEvent::ThinkingDelta(text));
            }
            return;
        }
        if let Some(signature) = part.thought_signature.filter(|_| self.thinking_open) {
            out.push(ProviderEvent::ThinkingSignature(format!(
                "{SIGNATURE_TAG}{signature}"
            )));
        }
        if let Some(text) = part.text.filter(|text| !text.is_empty()) {
            self.thinking_open = false;
            out.push(ProviderEvent::TextDelta(text));
        }
    }

    /// An error chunk: its message, its code read from its status and
    /// message, the chunk as its record, and the wait the standard reading
    /// finds.
    fn failure(&self, error: ChunkError, data: &str) -> ProviderFailure {
        let message = error
            .message
            .into_inner()
            .unwrap_or_else(|| "Google API stream error".to_owned());
        let status = error.status.into_inner();
        let failure = ProviderFailure {
            code: ErrorCode::classify(status.as_deref(), &message),
            message,
            diagnostics: Some(ProviderErrorDiagnostics {
                source: FailureSource::Stream,
                client_request_id: None,
                provider_request_id: None,
                provider_response_id: None,
                provider_code: status,
                http_status: None,
                upstream: Some(data.to_owned()),
            }),
            retry_after: None,
        };
        failure.with_retry_wait(read_http_failure, self.shared.clock.now())
    }
}

/// One `GenerateContentResponse` of the stream. Every field is optional, but
/// a field that is present holds what it claims.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chunk {
    #[serde(default)]
    candidates: Option<Vec<Candidate>>,
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
    #[serde(default)]
    error: Option<ChunkError>,
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<CandidateContent>,
}

#[derive(Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Option<Vec<ResponsePart>>,
}

/// A part of a candidate's content. Gemini parts carry no tag; their fields
/// tell them apart.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsePart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thought: Option<bool>,
    #[serde(default)]
    thought_signature: Option<String>,
    #[serde(default)]
    function_call: Option<FunctionCallPart>,
}

#[derive(Deserialize)]
struct FunctionCallPart {
    name: NonEmpty,
    #[serde(default)]
    args: Option<serde_json::Value>,
    #[serde(default)]
    id: Option<String>,
}

/// Token counts: whole numbers, or absent. Thinking is billed apart from the
/// answer, and both are output.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<u64>,
    #[serde(default)]
    candidates_token_count: Option<u64>,
    #[serde(default)]
    thoughts_token_count: Option<u64>,
    #[serde(default)]
    cached_content_token_count: Option<u64>,
}

impl UsageMetadata {
    fn token_usage(&self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.prompt_token_count.unwrap_or(0),
            output_tokens: self.candidates_token_count.unwrap_or(0)
                + self.thoughts_token_count.unwrap_or(0),
            cache_read_tokens: self.cached_content_token_count.unwrap_or(0),
            cache_write_tokens: 0,
        }
    }
}

#[derive(Deserialize)]
struct ChunkError {
    #[serde(default)]
    status: ReportedString,
    #[serde(default)]
    message: ReportedString,
}
