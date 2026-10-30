//! One run (`crates/provider-common/src/contract.rs`).
//!
//! 1. Resolve the account's credentials.
//! 2. Post the Responses request to the chat proxy.
//! 3. Map its stream.
//!
//! A request refused with HTTP 401 before any event refreshes the token that
//! was refused, once, and is sent again.

use std::sync::Arc;

use demi_provider_common::{
    CallEvent, ErrorCode, HttpFailureRecord, InferenceRequest, ProviderEvent, ProviderFailure, VendorCall, encode_body,
    http_failure,
    quota::Observation,
    read_http_failure,
    wire::{Vendor, responses},
};
use futures_util::{Stream, StreamExt};
use http::StatusCode;

use crate::{
    LABEL, SIGNATURE_TAG, Shared,
    request::{self, RequestIds},
};

/// A run of `request`. It checks its token before each step and races every
/// wait on the vendor against it; a cancelled run ends without a further
/// event, and dropping the stream drops the connection.
pub(crate) fn run(
    shared: Arc<Shared>,
    http: demi_provider_common::ProviderHttp,
    request: InferenceRequest,
) -> impl Stream<Item = ProviderEvent> {
    async_stream::stream! {
        let cancel = request.cancel.clone();
        if cancel.is_cancelled() {
            return;
        }
        let ids = RequestIds::of(&request);
        let model = request.model_id.clone();
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
            let requested_at = shared.clock.now();
            let send = http
                .post(shared.responses_url.clone())
                .headers(request::inference_headers(&credentials, &shared.releases, &ids))
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
            if let Some(observed) = shared.quota.observed(Observation::Response {
                status: response.status(),
                headers: response.headers(),
            }, requested_at) {
                yield observed;
            }
            if response.status() == StatusCode::UNAUTHORIZED && refused_token.is_none() {
                // The refused answer's body is not needed; dropping the
                // response releases its connection. The refused attempt is
                // a call of its own, and the next one starts.
                refused_token = Some(credentials.access_token);
                yield VendorCall::refused(Some(ErrorCode::AuthExpired));
                yield ProviderEvent::Call(CallEvent::Started);
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
                yield ProviderEvent::Error(with_usage_limit(failure));
            }
            return;
        }
        let vendor = Vendor {
            label: LABEL,
            reader: read_http_failure,
            clock: shared.clock.clone(),
            model,
        };
        let received = responses::sse_events(response.bytes_stream(), LABEL, cancel.clone());
        let mut events = std::pin::pin!(responses::map_events(received, vendor, SIGNATURE_TAG.into(), cancel));
        while let Some(event) = events.next().await {
            yield event;
        }
    }
}

/// `failure` with the code `usage_limit` when its answer is Grok Build's sign
/// of a used-up account, as the Grok CLI reads it.
///
/// - A 402 is one, whatever its body.
/// - A 403 whose text says the account ran out of credits is one.
/// - Any other 403 stays a refused request (`permission_denied`).
/// - A 429 only throttles.
pub(crate) fn with_usage_limit(mut failure: ProviderFailure) -> ProviderFailure {
    let used_up = failure
        .diagnostics
        .as_deref()
        .and_then(HttpFailureRecord::read)
        .is_some_and(|record| match record.status {
            402 => true,
            403 => record
                .body
                .to_ascii_lowercase()
                .contains("run out of credits"),
            _ => false,
        });
    if used_up {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}
