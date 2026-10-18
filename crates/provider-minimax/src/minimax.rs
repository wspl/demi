//! What every MiniMax request has in common
//! ([Error codes](https://platform.minimax.io/docs/api-reference/errorcode)):
//! the key as a bearer token, and two ways of refusing. The image, speech,
//! voice and v1 video endpoints answer HTTP 200 to a refused request, so
//! its `base_resp.status_code` decides: 0 is success, 1004 and 2049 a key
//! MiniMax does not know, 1008 a used-up balance, 1002 and 1039 a rate
//! limit. The v2 video endpoints answer with the HTTP status and an
//! `error` object, 401 for a refused key and 402 for a used-up balance.

use bytes::Bytes;
use demi_provider_common::{
    ErrorCode, HttpFailureRecord, ProviderFailure, ProviderHttp, Secret,
    media::{Ask, Holding, MediaAnswer, MediaCall, MediaCredential, MediaFailure, unreadable},
    read_http_failure,
};
use demi_shared_types::{Clock, VendorTrace};
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, HeaderMap},
};
use serde::Deserialize;

/// How failure records name MiniMax.
pub(crate) const LABEL: &str = "MiniMax";

/// How a MiniMax request is sent and its HTTP failures read.
pub(crate) fn call(clock: &dyn Clock) -> MediaCall<'_> {
    MediaCall {
        label: LABEL,
        credential: MediaCredential::Key { refused: refused_key },
        reader: read_http_failure,
        used_up,
        clock,
        quota: None,
    }
}

/// The headers of a request with `key`.
pub(crate) fn headers(key: &Secret) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, key.bearer());
    headers
}

/// Sends `ask` with `key` to an endpoint that refuses on HTTP 200, and
/// answers the body of its success, or the failure its `base_resp` states.
pub(crate) async fn send_checked(clock: &dyn Clock, http: &ProviderHttp, key: &Secret, ask: Ask) -> MediaAnswer<Bytes> {
    let headers = headers(key);
    call(clock)
        .send(http, ask, Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
        .await
        .and_then(|answered| base_resp(&answered.body, answered.trace_id).map(|()| answered.body))
}

/// MiniMax refuses a key it does not know with HTTP 401.
fn refused_key(status: StatusCode, _: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
}

/// A failure of HTTP 402, `insufficient_balance_error`.
fn used_up(mut failure: ProviderFailure) -> ProviderFailure {
    let record = failure.diagnostics.as_deref().and_then(HttpFailureRecord::read);
    if record.is_some_and(|record| record.status == StatusCode::PAYMENT_REQUIRED.as_u16()) {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}

/// The status of an answer MiniMax sent with HTTP 200, and what names the
/// request: speech's `trace_id`, an image's `id`.
#[derive(Deserialize)]
struct Status {
    #[serde(default)]
    base_resp: Option<BaseResp>,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct BaseResp {
    status_code: i64,
    #[serde(default)]
    status_msg: String,
}

/// The failure `body`'s `base_resp` states, if it states one, traced by
/// its code and the request's ID, the body's or else `trace_id`, the
/// answer's headers'; a body without it is the answer it documents, which
/// its reader reads.
pub(crate) fn base_resp(body: &[u8], trace_id: Option<String>) -> Result<(), MediaFailure> {
    let status: Status =
        serde_json::from_slice(body).map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    let Some(BaseResp { status_code, status_msg }) = status.base_resp else {
        return Ok(());
    };
    if status_code == 0 {
        return Ok(());
    }
    let trace = VendorTrace {
        code: Some(status_code.to_string()),
        id: status.trace_id.or(status.id).filter(|id| !id.is_empty()).or(trace_id),
    };
    let message = status_msg;
    Err(match status_code {
        1004 | 2049 => MediaFailure::KeyRefused { message, code: ErrorCode::AuthExpired, trace },
        1008 => MediaFailure::UsageLimit { retry_at: None, trace },
        1002 | 1039 => MediaFailure::Unanswered { cause: message, code: Some(ErrorCode::RateLimit), trace },
        1000 | 1001 | 1013 => MediaFailure::Unanswered { cause: message, code: Some(ErrorCode::Overloaded), trace },
        _ => MediaFailure::Refused { message, code: None, trace },
    })
}
