//! An HTTP failure and its standard reading (`failures-and-recovery.md` § The
//! failure record, § Reading a failure): the record that keeps the vendor's
//! whole answer, and the `Retry-After` header read as RFC 9110 defines it.

use std::time::SystemTime;

use demi_core::{
    Clock, FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{ErrorCode, FailureReader, ProviderFailure, failure::diagnostics};

/// What an HTTP failure keeps as its `upstream`: the status, every response
/// header as a `[name, value]` pair with the name in lowercase, sorted by
/// name with a repeated header kept as separate pairs in arrival order, and
/// the body text as received. Nothing in it is parsed, filtered or redacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpFailureRecord {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpFailureRecord {
    /// The record of a response, whatever order the HTTP stack reports its
    /// headers in.
    pub fn from_response(status: StatusCode, headers: &HeaderMap, body: String) -> Self {
        let mut pairs: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| {
                let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
                (name.as_str().to_owned(), value)
            })
            .collect();
        // A stable sort, so repeated headers keep the order they arrived in.
        pairs.sort_by(|left, right| left.0.cmp(&right.0));
        Self {
            status: status.as_u16(),
            headers: pairs,
            body,
        }
    }

    /// The record an HTTP failure's diagnostics hold; `None` for any other
    /// failure, or for a record that cannot be read.
    pub fn read(diagnostics: &ProviderErrorDiagnostics) -> Option<Self> {
        if diagnostics.source != FailureSource::Http {
            return None;
        }
        serde_json::from_str(diagnostics.upstream.as_deref()?).ok()
    }

    /// The first value of the header `name`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn to_json(&self) -> String {
        // A record is strings and a number, which always serialize.
        serde_json::to_string(self).expect("a failure record serializes")
    }
}

/// The failure of a response with a failure status: the vendor's text in the
/// message (`"{label} API request failed with HTTP {status}: {body}"`), the
/// whole response as the record, and the wait `reader` finds in it. A body
/// that cannot be read counts as empty.
pub async fn http_failure(
    response: reqwest::Response,
    label: &str,
    reader: FailureReader,
    clock: &dyn Clock,
) -> ProviderFailure {
    let status = response.status();
    let headers = response.headers().clone();
    // The status and headers still say what failed; the record keeps an
    // empty body.
    let body = response.text().await.unwrap_or_default();
    ProviderFailure::refused(label, status, &headers, body, reader, clock.now())
}

impl ProviderFailure {
    /// [`http_failure`] of an answer already read, such as a refused
    /// WebSocket handshake: the status, the headers and the body text, which
    /// arrived at `received_at`.
    pub fn refused(
        label: &str,
        status: StatusCode,
        headers: &HeaderMap,
        body: String,
        reader: FailureReader,
        received_at: Timestamp,
    ) -> Self {
        let message = if body.is_empty() {
            format!("{label} API request failed with HTTP {}", status.as_u16())
        } else {
            format!("{label} API request failed with HTTP {}: {body}", status.as_u16())
        };
        let record = HttpFailureRecord::from_response(status, headers, body);
        let failure = Self {
            code: ErrorCode::from_http(status.as_u16(), &message),
            message,
            diagnostics: Some(diagnostics(
                FailureSource::Http,
                Some(status.as_u16()),
                Some(record.to_json()),
            )),
            retry_after: None,
        };
        failure.with_retry_wait(reader, received_at)
    }
}

/// The standard reading of a failure record, which every provider can use:
/// the `Retry-After` header of an HTTP failure.
pub fn read_http_failure(
    diagnostics: &ProviderErrorDiagnostics,
    received_at: Timestamp,
) -> ProviderFailureFacts {
    let retry_at = HttpFailureRecord::read(diagnostics)
        .and_then(|record| retry_at(record.header("retry-after")?, received_at));
    ProviderFailureFacts { retry_at }
}

/// The moment a `Retry-After` value names (RFC 9110 § 10.2.3): a number of
/// seconds counted from `received_at`, with surrounding whitespace allowed
/// and a decimal fraction counted to the millisecond, or an HTTP-date. Any
/// other value names no time, such as `1e3`, `0x10`, an ISO 8601 date or
/// `-5`.
pub fn retry_at(value: &str, received_at: Timestamp) -> Option<Timestamp> {
    let value = value.trim_ascii();
    if let Some(delay) = delay_milliseconds(value) {
        let moment = received_at.as_millisecond().checked_add(delay)?;
        return Timestamp::from_millisecond(moment).ok();
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let since_epoch = date.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Timestamp::from_millisecond(i64::try_from(since_epoch.as_millis()).ok()?).ok()
}

/// The milliseconds of delta-seconds with an optional fraction, floored; `None`
/// for anything else, or for a delay too long to count.
fn delay_milliseconds(value: &str) -> Option<i64> {
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (value, None),
    };
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|fraction| !digits(fraction)) {
        return None;
    }
    let seconds: i64 = whole.parse().ok()?;
    let mut milliseconds = 0;
    for (place, digit) in fraction.unwrap_or_default().bytes().take(3).enumerate() {
        let scale = [100, 10, 1][place];
        milliseconds += i64::from(digit - b'0') * scale;
    }
    seconds.checked_mul(1000)?.checked_add(milliseconds)
}
