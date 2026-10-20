//! How the Codex provider reads its failure records: a used-up account is
//! told by Codex's own error type or code, as the Codex CLI tells it; a
//! usage limit says when it lifts, in the failure's own fields, and anything
//! else falls back to the standard reading.

use demi_provider_common::{
    ErrorCode, HttpFailureRecord, ProviderFailure, quota::unix_seconds, read_http_failure,
};
use demi_shared_types::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp};

/// The error type Codex refuses a request with when the account's plan
/// window is used up.
const USAGE_LIMIT_REACHED: &str = "usage_limit_reached";

/// `failure` with the code `usage_limit` when its record is Codex's sign of
/// a used-up account: a 429, over HTTP or in a WebSocket error frame, whose
/// error type is `usage_limit_reached`, or any failure, with a status or
/// none as in a `response.failed` frame, whose error type or code says the
/// quota, the credits or a spend or usage cap is used up. Any other failure
/// keeps its code.
pub(crate) fn with_usage_limit(mut failure: ProviderFailure) -> ProviderFailure {
    if failure.diagnostics.as_deref().is_some_and(is_usage_limit) {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}

fn is_usage_limit(diagnostics: &ProviderErrorDiagnostics) -> bool {
    let Some(failure) = failure(diagnostics) else {
        return false;
    };
    let Some(error) = failure.error else {
        return false;
    };
    let field = |name| error.get(name).and_then(serde_json::Value::as_str);
    let used_up = |value: &str| {
        matches!(value, "insufficient_quota" | "credit_balance_exhausted")
            || value.ends_with("_spend_limit_exceeded")
            || value.ends_with("_usage_limit_exceeded")
    };
    (failure.status == Some(429) && field("type") == Some(USAGE_LIMIT_REACHED))
        || field("type").is_some_and(used_up)
        || field("code").is_some_and(used_up)
}

/// A Codex failure as its record holds it, out of every envelope the
/// transport accepts: the HTTP status, or the status a WebSocket frame
/// names, and the error object.
struct Failure {
    status: Option<u64>,
    error: Option<serde_json::Map<String, serde_json::Value>>,
}

/// The failure a record holds: an HTTP failure's status and body, or a
/// stream frame. A frame may come in a WebSocket envelope, `{"event": …}`;
/// its error is the frame's `error`, or a failed response's
/// `response.error`; a WebSocket error frame names its status as `status`,
/// or as `status_code` as the Codex CLI also reads it. `None` for a record
/// that is not JSON.
fn failure(diagnostics: &ProviderErrorDiagnostics) -> Option<Failure> {
    let (status, text) = match diagnostics.source {
        FailureSource::Http => {
            let record = HttpFailureRecord::read(diagnostics)?;
            (Some(u64::from(record.status)), record.body)
        }
        _ => (None, diagnostics.upstream.clone()?),
    };
    let record: serde_json::Value = serde_json::from_str(&text).ok()?;
    let frame = record.get("event").filter(|event| event.is_object()).unwrap_or(&record);
    let status = status.or_else(|| {
        ["status", "status_code"]
            .iter()
            .find_map(|key| frame.get(key)?.as_u64())
    });
    let error = frame
        .get("error")
        .or_else(|| frame.get("response")?.get("error"))
        .and_then(serde_json::Value::as_object)
        .cloned();
    Some(Failure { status, error })
}

/// The facts of a Codex failure record: `resets_at`, a Unix time in seconds,
/// or `resets_in_seconds`, counted from when the failure was received, of
/// the failure's error object, out of every envelope (`failure`). A value that is not a number reads as
/// absent; without either, the standard `Retry-After` reading applies.
pub fn read_codex_failure(
    diagnostics: &ProviderErrorDiagnostics,
    received_at: Timestamp,
) -> ProviderFailureFacts {
    let limit = failure(diagnostics).and_then(|failure| failure.error);
    if let Some(limit) = limit {
        if let Some(resets_at) = limit.get("resets_at").and_then(serde_json::Value::as_f64) {
            return ProviderFailureFacts {
                retry_at: unix_seconds(resets_at),
                continue_with: None,
                signed_out: None,
                not_exposed: None,
                device_offline: None,
            };
        }
        if let Some(seconds) = limit
            .get("resets_in_seconds")
            .and_then(serde_json::Value::as_f64)
        {
            let lifts = received_at.as_millisecond() as f64 / 1000.0 + seconds;
            return ProviderFailureFacts {
                retry_at: unix_seconds(lifts),
                continue_with: None,
                signed_out: None,
                not_exposed: None,
                device_offline: None,
            };
        }
    }
    read_http_failure(diagnostics, received_at)
}
