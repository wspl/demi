//! A run's failure and its typed code (`failures-and-recovery.md`): what the
//! agent's retry policy reads, the vendor's record that the user and support
//! read, and the wait the vendor asked for.

use std::{
    convert::Infallible,
    error::Error as _,
    fmt,
    str::FromStr,
    sync::LazyLock,
    time::Duration,
};

use demi_core::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp};
use regex::Regex;

use crate::wire::SseError;

/// Reads a failure record a provider produced: the facts Demi shows, given
/// the moment the failure was received. The same reader sets a failure's wait
/// when a run fails and reads stored records when the backend shows them.
pub type FailureReader = fn(&ProviderErrorDiagnostics, Timestamp) -> ProviderFailureFacts;

/// A failed run: the run's last event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    /// What went wrong, as the vendor said it or as Demi composed it. A
    /// message Demi composes never contains a credential.
    pub message: String,
    /// What recovery reads; `None` for a failure Demi found in the vendor's
    /// answer, which is never retried automatically.
    pub code: Option<ErrorCode>,
    pub diagnostics: Option<ProviderErrorDiagnostics>,
    /// The wait the vendor asked for, counted from when the failure was
    /// received.
    pub retry_after: Option<Duration>,
}

impl ProviderFailure {
    /// A frame or line Demi could not decode, or one that breaks the vendor's
    /// protocol. It has no code, its source is `stream`, and its record is the
    /// text as received.
    pub fn protocol(message: impl Into<String>, received: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            diagnostics: Some(diagnostics(FailureSource::Stream, None, Some(received.into()))),
            retry_after: None,
        }
    }

    /// No answer from the vendor: the request could not be sent, or its
    /// response broke off. A network failure or a timeout is `overloaded`; a
    /// request that could not even be built has no code. The message names
    /// the error and its causes but never the endpoint, which stays inside
    /// the provider.
    pub fn transport(label: &str, error: reqwest::Error) -> Self {
        let code = if error.is_builder() {
            None
        } else {
            Some(ErrorCode::Overloaded)
        };
        let error = error.without_url();
        let mut message = format!("{label} API request failed: {error}");
        let mut cause = error.source();
        while let Some(inner) = cause {
            message.push_str(": ");
            message.push_str(&inner.to_string());
            cause = inner.source();
        }
        Self {
            message,
            code,
            diagnostics: Some(diagnostics(FailureSource::Transport, None, None)),
            retry_after: None,
        }
    }

    /// The failure of a response's event stream that could not be read: a
    /// body that broke off is a transport failure, and a body that is not an
    /// event stream is a protocol failure without a record.
    pub fn event_stream(label: &str, error: SseError<reqwest::Error>) -> Self {
        let message = match error {
            SseError::Transport(error) => return Self::transport(label, error),
            SseError::Utf8(error) => format!("{label} API stream is not UTF-8 text: {error}"),
            SseError::Syntax(error) => format!("{label} API stream cannot be parsed: {error}"),
        };
        Self {
            message,
            code: None,
            diagnostics: Some(diagnostics(FailureSource::Stream, None, None)),
            retry_after: None,
        }
    }

    /// Sets the wait the provider's `reader` finds in this failure's record,
    /// received at `now`. A failure without a record, or one whose reader names
    /// no time, keeps no wait; a time already past is no wait at all.
    pub fn with_retry_wait(mut self, reader: FailureReader, now: Timestamp) -> Self {
        let Some(diagnostics) = &self.diagnostics else {
            return self;
        };
        let Some(retry_at) = reader(diagnostics, now).retry_at else {
            return self;
        };
        let wait = retry_at.as_millisecond().saturating_sub(now.as_millisecond());
        self.retry_after = Some(Duration::from_millis(wait.max(0).unsigned_abs()));
        self
    }
}

/// Diagnostics with a source and a record and nothing else.
pub(crate) fn diagnostics(
    source: FailureSource,
    http_status: Option<u16>,
    upstream: Option<String>,
) -> ProviderErrorDiagnostics {
    ProviderErrorDiagnostics {
        source,
        client_request_id: None,
        provider_request_id: None,
        provider_response_id: None,
        provider_code: None,
        http_status,
        upstream,
    }
}

/// A failure's code, written as its string, such as `rate_limit`, wherever
/// it is stored or sent. The agent retries `rate_limit` and `overloaded`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// A quota or throttling failure.
    RateLimit,
    /// A transient failure: HTTP 5xx, a timeout, a network or socket failure.
    Overloaded,
    /// The request is larger than the model accepts.
    ContextLengthExceeded,
    /// The vendor ended the response before it was complete.
    Incomplete,
    /// The credential expired or was refused.
    AuthExpired,
    /// The account has no credential.
    AuthMissing,
    /// The stored credential cannot be read.
    AuthInvalid,
    /// The credential is of a kind the vendor's endpoint does not accept.
    AuthUnsupported,
    /// The credential could not be refreshed.
    AuthRefreshFailed,
    /// Any other failure the vendor named, with the vendor's own code.
    Vendor(String),
}

impl ErrorCode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::Overloaded => "overloaded",
            Self::ContextLengthExceeded => "context_length_exceeded",
            Self::Incomplete => "incomplete",
            Self::AuthExpired => "auth_expired",
            Self::AuthMissing => "auth_missing",
            Self::AuthInvalid => "auth_invalid",
            Self::AuthUnsupported => "auth_unsupported",
            Self::AuthRefreshFailed => "auth_refresh_failed",
            Self::Vendor(code) => code,
        }
    }

    /// The code of an HTTP failure status; `message` is the failure's text,
    /// which tells an oversized request among the 400s.
    pub fn from_http(status: u16, message: &str) -> Option<Self> {
        match status {
            401 | 403 => Some(Self::AuthExpired),
            429 => Some(Self::RateLimit),
            408 | 409 | 425 | 500.. => Some(Self::Overloaded),
            400 if TOO_LARGE.is_match(message) => Some(Self::ContextLengthExceeded),
            _ => None,
        }
    }

    /// The code of a failure the vendor reported inside a response, such as
    /// a stream's error event: its words decide a coarse category, and
    /// otherwise the vendor's own code stands. Only vendor-reported failures
    /// are classified; a failure Demi finds itself gets its code by a fixed
    /// rule.
    ///
    /// Words count only as whole words, with `_` and `-` separating them, so
    /// that `generate` is not `rate` and `unlimited` is not `limit`. `usage`
    /// alone is a quota failure, as in Codex's `usage_limit_reached`.
    pub fn classify(code: Option<&str>, message: &str) -> Option<Self> {
        let code = code.filter(|code| !code.is_empty());
        let text = words(&format!("{} {message}", code.unwrap_or_default()));
        let category = CATEGORIES
            .iter()
            .find(|(_, pattern)| pattern.is_match(&text))
            .map(|(category, _)| category.clone());
        category.or_else(|| {
            let Ok(code) = code?.parse();
            Some(code)
        })
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ErrorCode {
    type Err = Infallible;

    /// Reads a stored code: one of Demi's codes, or a vendor's own.
    fn from_str(code: &str) -> Result<Self, Infallible> {
        Ok(match code {
            "rate_limit" => Self::RateLimit,
            "overloaded" => Self::Overloaded,
            "context_length_exceeded" => Self::ContextLengthExceeded,
            "incomplete" => Self::Incomplete,
            "auth_expired" => Self::AuthExpired,
            "auth_missing" => Self::AuthMissing,
            "auth_invalid" => Self::AuthInvalid,
            "auth_unsupported" => Self::AuthUnsupported,
            "auth_refresh_failed" => Self::AuthRefreshFailed,
            other => Self::Vendor(other.to_owned()),
        })
    }
}

/// A 400 whose text says the request is too large for the model.
static TOO_LARGE: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?i)context|too long|token"));

/// The categories a vendor's failure text is read into, in the order they
/// are tried, over [`words`].
static CATEGORIES: LazyLock<[(ErrorCode, Regex); 4]> = LazyLock::new(|| {
    [
        (
            ErrorCode::ContextLengthExceeded,
            pattern(r"\bcontext\b|\btoo long\b|\bmax\w*\b.*\btokens?\b"),
        ),
        (
            ErrorCode::RateLimit,
            pattern(r"\brate\b|\bratelimit\w*|\bquota\b|\busage\b|\bbilling\b|\bbalance\b"),
        ),
        (
            ErrorCode::AuthExpired,
            pattern(concat!(
                r"\bauth(?:entication|orization)?\b",
                r"|(?:invalid|expired).*(?:api|access|auth) ?(?:key|token)",
                r"|(?:api|access|auth) ?(?:key|token).*(?:invalid|expired)",
            )),
        ),
        (
            ErrorCode::Overloaded,
            pattern(concat!(
                r"\boverload|\bunavailable\b|\b(?:server|internal|api) error\b",
                r"|\btimed? ?out|\bfetch failed\b|\bnetwork\b|\bsocket\b|\beconn",
            )),
        ),
    ]
});

fn pattern(source: &str) -> Regex {
    // The patterns are constants of this module; a test compiles every one.
    Regex::new(source).expect("a classification pattern compiles")
}

/// `text` as lowercase ASCII words separated by single spaces: every other
/// character separates words, so `rate_limit_error` reads as
/// `rate limit error`.
fn words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut separated = true;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
            separated = false;
        } else if !separated {
            out.push(' ');
            separated = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}
