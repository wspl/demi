//! A run's failure and its typed code: what the agent's retry policy reads,
//! the vendor's record that the user and support read, and the wait the
//! vendor asked for.

use std::{convert::Infallible, error::Error as _, str::FromStr, sync::LazyLock, time::Duration};

use demi_shared_types::{FailureSource, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp};
use regex::Regex;

use crate::client::{DeviceRefusal, SendError};
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
    pub diagnostics: Option<Box<ProviderErrorDiagnostics>>,
    /// The wait the vendor asked for, counted from when the failure was
    /// received.
    pub retry_after: Option<Duration>,
}

/// What a request that did not reach the vendor, or broke off, failed
/// with: the error and each of its causes, without the URL; a device's
/// refusal in its own words.
pub(crate) fn transport_cause(error: impl Into<SendError>) -> String {
    let error = match error.into() {
        SendError::Transport(error) => error.without_url(),
        SendError::Device(refusal) => return refusal.to_string(),
    };
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        cause = inner.source();
    }
    message
}

impl ProviderFailure {
    /// A frame or line Demi could not decode, or one that breaks the vendor's
    /// protocol. It has no code, its source is `stream`, and its record is the
    /// text as received.
    pub fn protocol(message: impl Into<String>, received: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            diagnostics: Some(Box::new(diagnostics(
                FailureSource::Stream,
                None,
                Some(received.into()),
            ))),
            retry_after: None,
        }
    }

    /// No answer from the vendor: the request could not be sent, or its
    /// response broke off. A network failure or a timeout is `overloaded`; a
    /// request that could not even be built has no code. The message names
    /// the error and its causes but never the endpoint, which stays inside
    /// the provider.
    ///
    /// A request to a device's port fails in Demi's words: a port the
    /// device has not exposed with `port_not_exposed`, which waiting cannot
    /// fix, an offline device with `device_offline`, retried as an
    /// unreachable endpoint is, and a failed request as any unreachable
    /// endpoint (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
    pub fn transport(label: &str, error: impl Into<SendError>) -> Self {
        let error = match error.into() {
            SendError::Transport(error) => error,
            SendError::Device(refusal @ DeviceRefusal::NotExposed { .. }) => {
                return Self {
                    code: Some(ErrorCode::PortNotExposed),
                    ..Self::no_answer(refusal.to_string())
                };
            }
            SendError::Device(refusal @ DeviceRefusal::Offline { .. }) => {
                return Self {
                    code: Some(ErrorCode::DeviceOffline),
                    ..Self::no_answer(refusal.to_string())
                };
            }
            SendError::Device(DeviceRefusal::Failed(cause)) => {
                return Self::no_answer(format!("{label} API request failed: {cause}"));
            }
        };
        let unbuilt = error.is_builder();
        let message = format!("{label} API request failed: {}", transport_cause(error));
        let failure = Self::no_answer(message);
        if unbuilt {
            return Self {
                code: None,
                ..failure
            };
        }
        failure
    }

    /// No answer from the vendor, in Demi's words, such as a connection that
    /// timed out: `overloaded`, from the source `transport`, with no record.
    pub fn no_answer(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: Some(ErrorCode::Overloaded),
            diagnostics: Some(Box::new(diagnostics(FailureSource::Transport, None, None))),
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
            diagnostics: Some(Box::new(diagnostics(FailureSource::Stream, None, None))),
            retry_after: None,
        }
    }

    /// A stream that ended before the vendor said the reply was complete,
    /// even with a usage count seen (`crates/provider-common/src/wire/mod.rs`):
    /// no answer, as any network failure is.
    pub fn cut_short() -> Self {
        Self::no_answer("The provider's stream ended before the reply was complete")
    }

    /// A reply the vendor ended before it was complete, for `end`, with the
    /// vendor's own `reason` in the message and as the provider code, and
    /// the frame that said so, `received`, as the record: `incomplete`.
    pub fn ended_early(end: EarlyEnd, reason: &str, received: impl Into<String>) -> Self {
        let message = match end {
            EarlyEnd::OutputLimit => format!("The reply reached the model's output limit ({reason})"),
            EarlyEnd::Filter => format!("The provider's filter stopped the reply ({reason})"),
            EarlyEnd::Other => {
                format!("The provider ended the reply before it was complete ({reason})")
            }
        };
        let mut diagnostics = diagnostics(FailureSource::Stream, None, Some(received.into()));
        diagnostics.provider_code = Some(reason.to_owned());
        Self {
            message,
            code: Some(ErrorCode::Incomplete),
            diagnostics: Some(Box::new(diagnostics)),
            retry_after: None,
        }
    }

    /// Sets the wait the provider's `reader` finds in this failure's record,
    /// received at `now`. A failure without a record, or one whose reader names
    /// no time, keeps no wait; a time already past is no wait at all.
    pub fn with_retry_wait(
        mut self,
        reader: impl FnOnce(&ProviderErrorDiagnostics, Timestamp) -> ProviderFailureFacts,
        now: Timestamp,
    ) -> Self {
        let Some(diagnostics) = &self.diagnostics else {
            return self;
        };
        let Some(retry_at) = reader(diagnostics, now).retry_at else {
            return self;
        };
        let wait = retry_at
            .as_millisecond()
            .saturating_sub(now.as_millisecond());
        self.retry_after = Some(Duration::from_millis(wait.max(0).unsigned_abs()));
        self
    }
}

/// Why a vendor ended a reply before it was complete; each provider reads
/// its vendor's reason into one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarlyEnd {
    /// The reply reached the model's output limit.
    OutputLimit,
    /// A filter of the vendor's stopped the reply, or the model refused.
    Filter,
    /// Any other reason the vendor named.
    Other,
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
    /// The account cannot be used until a reset or a change of its plan: a
    /// used-up subscription window, credits or a spend cap, or use its
    /// administrator turned off. Waiting a few seconds cannot fix it, so the
    /// agent does not retry it.
    UsageLimit,
    /// Throttling: too many requests or tokens for a short while.
    RateLimit,
    /// The backend's own request rate limit refused the attempt before it
    /// reached the vendor (`crates/backend-providers/src/usage/rate_limit.rs`); unlike
    /// `rate_limit`, the agent does not retry it by itself.
    RateLimited,
    /// A transient failure: HTTP 5xx, a timeout, a network or socket failure.
    Overloaded,
    /// The request holds more tokens than the model accepts.
    ContextLengthExceeded,
    /// The request's body or its number of images is more than the vendor
    /// accepts, however few tokens it holds.
    RequestTooLarge,
    /// The vendor ended the reply before it was complete: for its output
    /// limit, a filter or another reason it named.
    Incomplete,
    /// The vendor does not accept the credential: it expired, or the key or
    /// token is not one the vendor knows (HTTP 401).
    AuthExpired,
    /// The vendor knows the credential and refuses this request with it, as
    /// a plan without API access or a model the account may not use (HTTP
    /// 403); signing in again changes nothing, so it is not retried.
    PermissionDenied,
    /// The account has no credential.
    AuthMissing,
    /// The stored credential cannot be read.
    AuthInvalid,
    /// The credential is of a kind the vendor's endpoint does not accept.
    AuthUnsupported,
    /// The credential could not be refreshed.
    AuthRefreshFailed,
    /// The request named a port of one of the user's devices that the
    /// device has not exposed (`crates/backend-user-shard/src/shard/exposed_ports.rs`): the failure offers
    /// Expose Port, and the agent does not retry it.
    PortNotExposed,
    /// The request named a port of one of the user's devices while the
    /// device is offline, or a Cloud stopped (`crates/backend-user-shard/src/shard/exposed_ports.rs`): retried
    /// as an unreachable endpoint is, and the failure names the device.
    DeviceOffline,
    /// Any other failure the vendor named, with the vendor's own code.
    Vendor(String),
}

impl ErrorCode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::UsageLimit => "usage_limit",
            Self::RateLimit => "rate_limit",
            Self::RateLimited => "rate_limited",
            Self::Overloaded => "overloaded",
            Self::ContextLengthExceeded => "context_length_exceeded",
            Self::RequestTooLarge => "request_too_large",
            Self::Incomplete => "incomplete",
            Self::AuthExpired => "auth_expired",
            Self::PermissionDenied => "permission_denied",
            Self::AuthMissing => "auth_missing",
            Self::AuthInvalid => "auth_invalid",
            Self::AuthUnsupported => "auth_unsupported",
            Self::AuthRefreshFailed => "auth_refresh_failed",
            Self::PortNotExposed => "port_not_exposed",
            Self::DeviceOffline => "device_offline",
            Self::Vendor(code) => code,
        }
    }

    /// The code of an HTTP failure status; `message` is the failure's text,
    /// which tells a request refused for its size, and one refused for its
    /// tokens, among the 400s. A body the vendor refuses for its size (413)
    /// is too large, not too long: leaving media out makes it smaller.
    pub fn from_http(status: u16, message: &str) -> Option<Self> {
        match status {
            401 => Some(Self::AuthExpired),
            403 => Some(Self::PermissionDenied),
            429 => Some(Self::RateLimit),
            408 | 409 | 425 | 500.. => Some(Self::Overloaded),
            413 => Some(Self::RequestTooLarge),
            400 if TOO_LARGE.is_match(message) => Some(Self::RequestTooLarge),
            400 if TOO_LONG.is_match(message) => Some(Self::ContextLengthExceeded),
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
    /// that `generate` is not `rate`. `limit` and `usage` decide nothing: a
    /// throttled request's text may say *not your usage limit*, and a
    /// vendor's own sign of a used-up account is read by its provider.
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

impl FromStr for ErrorCode {
    type Err = Infallible;

    /// Reads a stored code: one of Demi's codes, or a vendor's own.
    fn from_str(code: &str) -> Result<Self, Infallible> {
        Ok(match code {
            "usage_limit" => Self::UsageLimit,
            "rate_limit" => Self::RateLimit,
            "rate_limited" => Self::RateLimited,
            "overloaded" => Self::Overloaded,
            "context_length_exceeded" => Self::ContextLengthExceeded,
            "request_too_large" => Self::RequestTooLarge,
            "incomplete" => Self::Incomplete,
            "auth_expired" => Self::AuthExpired,
            "permission_denied" => Self::PermissionDenied,
            "auth_missing" => Self::AuthMissing,
            "auth_invalid" => Self::AuthInvalid,
            "auth_unsupported" => Self::AuthUnsupported,
            "auth_refresh_failed" => Self::AuthRefreshFailed,
            "port_not_exposed" => Self::PortNotExposed,
            "device_offline" => Self::DeviceOffline,
            other => Self::Vendor(other.to_owned()),
        })
    }
}

/// A 400 whose text says the request's body or images are more than the
/// vendor takes, such as the Anthropic API's refusals of a request with more
/// images than it takes, or with more than 20 images when one exceeds its
/// size for many-image requests. It is tried before [`TOO_LONG`], which
/// `too large` alone matches.
static TOO_LARGE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)too many images|many-image|request too large|payload too large")
});

/// A 400 whose text says the request holds more tokens than the model takes.
static TOO_LONG: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)context|too long|token|too large"));

/// The categories a vendor's failure text is read into, in the order they
/// are tried, over [`words`].
static CATEGORIES: LazyLock<[(ErrorCode, Regex); 6]> = LazyLock::new(|| {
    [
        (
            ErrorCode::RequestTooLarge,
            pattern(
                r"\btoo many images\b|\bmany image\b|\brequest too large\b|\bpayload too large\b",
            ),
        ),
        (
            ErrorCode::ContextLengthExceeded,
            pattern(r"\bcontext\b|\btoo long\b|\btoo large\b|\bmax\w*\b.*\btokens?\b"),
        ),
        (
            ErrorCode::UsageLimit,
            pattern(r"\bbilling\b|\bbalance\b|\bout of credits\b"),
        ),
        // `quota` throttles: Gemini's per-minute RESOURCE_EXHAUSTED says it.
        (
            ErrorCode::RateLimit,
            pattern(r"\bquota\b|\brate\b|\bratelimit\w*"),
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

/// The request ID a failure's message quotes, as OpenAI's `request ID
/// req_…` and Ark's `Request ID: 0217…` do, which they ask users to quote.
pub fn request_id_in(message: &str) -> Option<String> {
    static REQUEST_ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        // A constant of this module; a test reads a message through it.
        regex::Regex::new(r"(?i)request ID:? ([A-Za-z0-9_-]+)").expect("the request id pattern compiles")
    });
    REQUEST_ID.captures(message).map(|captures| captures[1].to_owned())
}
