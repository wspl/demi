//! One run (`providers.md` § A run): resolve the account's credentials, post
//! the Chat Completions request to the chat proxy, and map its stream. A
//! request refused with HTTP 401 before any event refreshes the token that
//! was refused, once, and is sent again.

use std::sync::Arc;

use demi_provider::{
    InferenceRequest, ProviderEvent, ProviderFailure, encode_body, http_failure,
    quota::Observation,
    read_http_failure,
    wire::{Vendor, chat_completions},
};
use futures_util::{Stream, StreamExt};
use http::StatusCode;

use crate::{
    LABEL, Shared,
    request::{self, RequestIds},
};

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
        let ids = RequestIds::of(&request);
        let body = encode_body(LABEL, move || request::encode(&request));
        let body = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            body = body => Some(body),
        };
        let body = match body {
            None => return,
            // Shareable bytes, sent again as they are after a refresh.
            Some(Ok(body)) => bytes::Bytes::from(body),
            Some(Err(failure)) => {
                yield ProviderEvent::Error(failure);
                return;
            }
        };
        // The access token of a request refused with HTTP 401, which is sent
        // again once with refreshed credentials.
        let mut refused_token = None;
        let response = loop {
            let credentials = tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                credentials = shared.auth.credentials(&http, refused_token.as_ref()) => credentials,
            };
            let credentials = match credentials {
                Ok(credentials) => credentials,
                Err(failure) => {
                    yield ProviderEvent::Error(failure.failure());
                    return;
                }
            };
            let send = http
                .post(shared.chat_url.clone())
                .headers(request::inference_headers(&credentials, &ids))
                .body(body.clone())
                .send();
            let sent = tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                sent = send => sent,
            };
            let response = match sent {
                Ok(response) => response,
                Err(error) => {
                    yield ProviderEvent::Error(ProviderFailure::transport(LABEL, error));
                    return;
                }
            };
            shared.quota.observe(Observation::Response {
                status: response.status(),
                headers: response.headers(),
            });
            if response.status() == StatusCode::UNAUTHORIZED && refused_token.is_none() {
                // The refused answer's body is not needed; dropping the
                // response releases its connection.
                refused_token = Some(credentials.access_token);
                continue;
            }
            break response;
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
        let mut events = std::pin::pin!(chat_completions::map_sse(response.bytes_stream(), vendor, cancel));
        while let Some(event) = events.next().await {
            yield event;
        }
    }
}
