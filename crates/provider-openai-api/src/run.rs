//! One run: the request, then the endpoint's event stream mapped onto
//! provider events by the shared mapper of its wire (`providers.md` § A run).

use std::sync::Arc;

use demi_core::WireApi;
use demi_provider::{
    InferenceRequest, ProviderEvent, ProviderFailure, encode_body, http_failure, read_http_failure,
    wire::{Vendor, chat_completions, responses},
};
use futures_util::{Stream, StreamExt};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};

use crate::{SIGNATURE_TAG, Shared, request};

/// How failure messages name the vendor.
const LABEL: &str = "OpenAI";

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
        let wire = shared.wire;
        let policy = shared.policy;
        let body = encode_body(LABEL, move || match wire {
            WireApi::Responses => request::responses(&request, policy),
            WireApi::ChatCompletions => request::chat_completions(&request, policy),
        });
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
            .post(shared.url.clone())
            .header(AUTHORIZATION, shared.authorization.clone())
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
        let vendor = Vendor {
            label: LABEL,
            reader: read_http_failure,
            clock: shared.clock.clone(),
        };
        let body = response.bytes_stream();
        let events = match wire {
            WireApi::Responses => {
                let received = responses::sse_events(body, LABEL, cancel.clone());
                responses::map_events(received, vendor, SIGNATURE_TAG, cancel).boxed_local()
            }
            WireApi::ChatCompletions => chat_completions::map_sse(body, vendor, cancel).boxed_local(),
        };
        let mut events = events;
        while let Some(event) = events.next().await {
            yield event;
        }
    }
}
