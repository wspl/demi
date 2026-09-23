//! One run (`providers.md` § A run): resolve the account's credentials, open
//! the request by the provider's transport, and map the Responses events.
//! A request refused with HTTP 401 before any event refreshes the token that
//! was refused, once, and is sent again.

use std::sync::Arc;

use demi_provider::{
    InferenceRequest, ProviderEvent, ProviderFailure, encode_body,
    wire::{ReportedString, Vendor, responses},
};
use futures_util::{Stream, StreamExt};
use http::StatusCode;
use serde::Deserialize;

use crate::{
    LABEL, SIGNATURE_TAG, Shared, TransportMode,
    failure::read_codex_failure,
    request,
    transport::{self, OpenError, Refusal},
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
        let session_id = request.session_id.clone();
        let request_id = request.request_id.clone();
        let websocket = shared.transport != TransportMode::Sse;
        let body = encode_body(LABEL, move || request::encode(&request, websocket));
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
        // The access token of a request refused with HTTP 401, which is sent
        // again once with refreshed credentials.
        let mut refused_token = None;
        let events = loop {
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
            let headers = request::inference_headers(&credentials, &shared.user_agent, &session_id, &request_id);
            let opened = tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                opened = transport::open(&shared, &http, &headers, &body, &cancel) => opened,
            };
            match opened {
                Ok(events) => break events,
                Err(OpenError::Refused(refusal)) if refusal.status == StatusCode::UNAUTHORIZED && refused_token.is_none() => {
                    refused_token = Some(credentials.access_token);
                }
                Err(OpenError::Refused(refusal)) => {
                    yield ProviderEvent::Error(refused(refusal, &shared));
                    return;
                }
                Err(OpenError::Failed(failure)) => {
                    yield ProviderEvent::Error(failure);
                    return;
                }
            }
        };
        let vendor = Vendor {
            label: LABEL,
            reader: read_codex_failure,
            clock: shared.clock.clone(),
        };
        let mut events = std::pin::pin!(responses::map_events(events, vendor, SIGNATURE_TAG, cancel));
        while let Some(event) = events.next().await {
            yield event;
        }
    }
}

/// The failure of a refused request: the whole answer as its record, the
/// wait the Codex reader finds in it, and the vendor's code and request id.
fn refused(refusal: Refusal, shared: &Shared) -> ProviderFailure {
    let body: ErrorBody = serde_json::from_str(&refusal.body).unwrap_or_default();
    let error = body.error.into_inner().unwrap_or_default();
    let provider_code = error.code.into_inner().or_else(|| error.kind.into_inner());
    let header_request_id = refusal
        .headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let provider_request_id = header_request_id.or_else(|| body.request_id.into_inner());
    let mut failure = ProviderFailure::refused(
        LABEL,
        refusal.status,
        &refusal.headers,
        refusal.body,
        read_codex_failure,
        shared.clock.now(),
    );
    if let Some(diagnostics) = &mut failure.diagnostics {
        diagnostics.provider_code = provider_code;
        diagnostics.provider_request_id = provider_request_id;
    }
    failure
}

/// The body of a refused request, as far as Demi reports it; a body it
/// cannot read leaves the status to speak.
#[derive(Default, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: demi_provider::wire::Reported<ErrorFields>,
    #[serde(default)]
    request_id: ReportedString,
}

#[derive(Default, Deserialize)]
struct ErrorFields {
    #[serde(default)]
    code: ReportedString,
    #[serde(default, rename = "type")]
    kind: ReportedString,
}
