//! Reading a failure of the CLI's (`failures-and-recovery.md` § Reading a
//! failure): a subscription's usage limit names its reset in the refused
//! call's own `rate_limit_info`, which the CLI prints on the assistant line
//! it makes up for the refusal; any other failure is read by the HTTP rules.

use demi_provider_common::quota::unix_seconds;
use demi_provider_common::read_http_failure;
use demi_provider_common::wire::ReportedString;
use demi_shared_types::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp};
use serde::Deserialize;

/// The `api_error` the CLI names a call with that its account's own usage
/// limit refused (Claude Code 2.1.296): a used-up session, weekly or
/// per-model window, used-up usage credits, or a seat or member whose use is
/// turned off. The CLI sets it from the refusal's unified rate-limit
/// headers and never for a 429 that only throttles, so it, not the status
/// or the words, tells a usage limit.
pub(crate) const USAGE_LIMIT: &str = "usage_limit_reached";

/// What a `result` line says of the API error that ended the turn.
#[derive(Deserialize)]
struct ResultRecord {
    #[serde(default)]
    api_error: ReportedString,
}

/// The facts of a failure this provider produced, received at
/// `received_at`: a usage limit can succeed again at the reset its refused
/// call names; any other failure, or a limit that names no reset, such as a
/// seat whose usage credits are turned off, is read by the HTTP rules.
pub(crate) fn read_failure(
    diagnostics: &ProviderErrorDiagnostics,
    received_at: Timestamp,
) -> ProviderFailureFacts {
    match usage_limit_reset(diagnostics) {
        Some(reset) => ProviderFailureFacts {
            retry_at: Some(reset),
            continue_with: None,
            signed_out: None,
            not_exposed: None,
            device_offline: None,
        },
        None => read_http_failure(diagnostics, received_at),
    }
}

/// The reset of a usage limit's record: the failing assistant line and the
/// `result` line, joined by a newline as received, when the result's API
/// error is the usage limit; the moment is the earlier of the assistant
/// line's `api_error_params.rate_limit_info.resetsAt`, the window's reset,
/// and its `overageResetsAt`, the usage credits' reset, of those present,
/// in Unix seconds, since either lifts the limit. It comes
/// before the account's quota, which may be older than the refusal; the
/// quota stands in only for a limit that names no reset, which the backend
/// reads for any provider.
fn usage_limit_reset(diagnostics: &ProviderErrorDiagnostics) -> Option<Timestamp> {
    if diagnostics.source != FailureSource::Stream {
        return None;
    }
    let (notice, result) = diagnostics.upstream.as_deref()?.rsplit_once('\n')?;
    let result: ResultRecord = serde_json::from_str(result).ok()?;
    if result.api_error.as_deref() != Some(USAGE_LIMIT) {
        return None;
    }
    let notice: serde_json::Value = serde_json::from_str(notice).ok()?;
    let info = notice.get("api_error_params")?.get("rate_limit_info")?;
    ["resetsAt", "overageResetsAt"]
        .iter()
        .filter_map(|key| unix_seconds(info.get(key)?.as_f64()?))
        .min()
}
