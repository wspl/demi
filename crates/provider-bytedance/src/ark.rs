//! What every Ark request has in common, Seedream's and Seedance's alike ([Base
//! URL and
//! authentication](https://docs.byteplus.com/en/docs/modelark/base-url-and-authentication),
//! [Error codes](https://docs.byteplus.com/en/docs/modelark/error-codes)).
//!
//! - The key is a bearer token.
//! - A key Ark does not know is refused with HTTP 401 `AuthenticationError`.
//! - An account whose balance is overdue is refused with HTTP 403, which is a
//!   used-up key, not a refused one.

use demi_provider_common::{
    ErrorCode, HttpFailureRecord, ProviderFailure, Secret,
    media::{MediaCall, MediaCredential},
    read_http_failure,
};
use demi_shared_types::Clock;
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, HeaderMap},
};
use serde::Deserialize;

/// How failure records name Ark.
const LABEL: &str = "Ark";

/// The codes of an account Ark stops for its unpaid balance.
const OVERDUE: [&str; 2] = ["AccountOverdueError", "OperationDenied.ServiceOverdue"];

/// How an Ark request is sent and its failures read.
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

/// The headers of a request with `key`, none for a compatible endpoint
/// that takes none.
pub(crate) fn headers(key: Option<&Secret>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(key) = key {
        headers.insert(AUTHORIZATION, key.bearer());
    }
    headers
}

/// Ark refuses a key it does not know with HTTP 401.
fn refused_key(status: StatusCode, _: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
}

/// A failure whose `error.code` is an overdue account's.
fn used_up(mut failure: ProviderFailure) -> ProviderFailure {
    #[derive(Deserialize)]
    struct Refusal {
        error: Detail,
    }
    #[derive(Deserialize)]
    struct Detail {
        code: String,
    }
    let refusal = failure
        .diagnostics
        .as_deref()
        .and_then(HttpFailureRecord::read)
        .and_then(|record| serde_json::from_str::<Refusal>(&record.body).ok());
    if refusal.is_some_and(|refusal| OVERDUE.contains(&refusal.error.code.as_str())) {
        failure.code = Some(ErrorCode::UsageLimit);
    }
    failure
}
