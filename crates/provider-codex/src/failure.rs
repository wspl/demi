//! How the Codex provider reads its failure records
//! (`failures-and-recovery.md` § Reading a failure): a usage limit says when
//! it lifts, in the failure's own fields, and anything else falls back to
//! the standard reading.

use demi_core::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp};
use demi_provider::{HttpFailureRecord, quota::unix_seconds, read_http_failure};

/// The facts of a Codex failure record: `resets_at`, a Unix time in seconds,
/// or `resets_in_seconds`, counted from when the failure was received, of
/// the first error object among a stream event's `error`, a WebSocket
/// envelope's `event.error`, a failed response's `response.error`, or an
/// HTTP failure body's `error`. A value that is not a number reads as
/// absent; without either, the standard `Retry-After` reading applies.
pub fn read_codex_failure(
    diagnostics: &ProviderErrorDiagnostics,
    received_at: Timestamp,
) -> ProviderFailureFacts {
    let text = match diagnostics.source {
        FailureSource::Http => HttpFailureRecord::read(diagnostics).map(|record| record.body),
        _ => diagnostics.upstream.clone(),
    };
    let limit = text
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|failure| usage_limit(&failure).cloned());
    if let Some(limit) = limit {
        if let Some(resets_at) = limit.get("resets_at").and_then(serde_json::Value::as_f64) {
            return ProviderFailureFacts {
                retry_at: unix_seconds(resets_at),
            };
        }
        if let Some(seconds) = limit
            .get("resets_in_seconds")
            .and_then(serde_json::Value::as_f64)
        {
            let lifts = received_at.as_millisecond() as f64 / 1000.0 + seconds;
            return ProviderFailureFacts {
                retry_at: unix_seconds(lifts),
            };
        }
    }
    read_http_failure(diagnostics, received_at)
}

/// The error object a Codex failure carries its usage limit in.
fn usage_limit(failure: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let nested = |outer: &str| failure.get(outer)?.get("error")?.as_object();
    failure
        .get("error")
        .and_then(serde_json::Value::as_object)
        .or_else(|| nested("event"))
        .or_else(|| nested("response"))
}
