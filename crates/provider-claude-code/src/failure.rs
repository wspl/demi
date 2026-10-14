//! Reading a failure of the CLI's (`failures-and-recovery.md` § Reading a
//! failure): a subscription's usage limit names no wait, only words such as
//! "You've hit your session limit · resets 9am (UTC)", so the moment it can
//! succeed again is when the account's limited window resets, as the
//! account's quota says; any other failure is read by the HTTP rules.

use demi_provider_common::read_http_failure;
use demi_provider_common::wire::ReportedString;
use demi_shared_types::{
    FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, QuotaSnapshot, Timestamp,
};
use serde::Deserialize;

use crate::Shared;

/// The `api_error` the CLI's `result` line names a turn its account's own
/// usage limit refused with, a plan's session, weekly or per-model limit
/// (Claude Code 2.1.296): the typed cause the CLI gives for consumers that
/// key on it rather than on its words, which may change.
const USAGE_LIMIT: &str = "usage_limit_reached";

/// What a `result` line says of the API error that ended the turn.
#[derive(Deserialize)]
struct ResultRecord {
    #[serde(default)]
    api_error: ReportedString,
}

impl Shared {
    /// The facts of a failure this provider produced, received at
    /// `received_at`: a usage limit can succeed again when the account's
    /// limited window resets, and any other failure, or a limit whose reset
    /// the quota does not know, is read by the HTTP rules.
    pub(crate) fn read_failure(
        &self,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> ProviderFailureFacts {
        let reset = is_usage_limit(diagnostics)
            .then(|| self.quota.latest())
            .flatten()
            .and_then(|snapshot| limited_reset(&snapshot, received_at));
        match reset {
            Some(reset) => ProviderFailureFacts {
                retry_at: Some(reset),
            },
            None => read_http_failure(diagnostics, received_at),
        }
    }
}

/// Whether the failure's record is a `result` line whose API error is the
/// account's usage limit. A record that is not such a line is not one.
fn is_usage_limit(diagnostics: &ProviderErrorDiagnostics) -> bool {
    if diagnostics.source != FailureSource::Stream {
        return false;
    }
    let Some(upstream) = &diagnostics.upstream else {
        return false;
    };
    serde_json::from_str::<ResultRecord>(upstream)
        .is_ok_and(|record| record.api_error.as_deref() == Some(USAGE_LIMIT))
}

/// When the account's limited windows reset: the windows the snapshot says
/// are used up, whose reset is still to come at `received_at`; with several,
/// the latest, since the request succeeds only once every one has reset.
fn limited_reset(snapshot: &QuotaSnapshot, received_at: Timestamp) -> Option<Timestamp> {
    snapshot
        .windows
        .iter()
        .filter(|window| window.used_percent.is_some_and(|used| used >= 100.0))
        .filter_map(|window| window.resets_at)
        .filter(|reset| *reset > received_at)
        .max()
}
