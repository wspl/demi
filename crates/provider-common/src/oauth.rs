//! The pieces every OAuth device login and token refresh meets
//! (`crates/backend-providers/src/vault/logins.rs`,
//! `crates/provider-common/src/wire/mod.rs`).
//!
//! They are the durations servers state as numbers or as digits, a response
//! decoded without quoting its tokens, and the claims read from a token. The
//! flows themselves are each vendor's, written by hand on these: the `oauth2`
//! crate refuses the vendors' token responses, which omit `token_type` and
//! state `expires_in` as a string.

use std::{sync::LazyLock, time::Duration};

use base64::{
    Engine,
    alphabet::URL_SAFE,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use regex::Regex;
use serde::{Deserialize, Deserializer, de, de::DeserializeOwned};

use crate::credentials::{SecretDecodeError, decode_secret};
use crate::wire::ReportedString;

/// A duration in seconds as an OAuth server states it: RFC 8628 says a
/// number, and some deployments send the digits as a string. The number is
/// finite and not negative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OAuthSeconds(pub f64);

impl OAuthSeconds {
    pub fn duration(self) -> Duration {
        Duration::from_secs_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for OAuthSeconds {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        seconds(&serde_json::Value::deserialize(deserializer)?)
            .map(Self)
            .ok_or_else(|| de::Error::custom("expected a number of seconds"))
    }
}

/// The seconds a value states: a number, or a string of digits with an
/// optional fraction; finite and not negative.
fn seconds(value: &serde_json::Value) -> Option<f64> {
    static DIGITS: LazyLock<Regex> = LazyLock::new(|| {
        // A constant of this module; the OAuth tests read strings through it.
        Regex::new(r"^\d+(\.\d+)?$").expect("the seconds pattern compiles")
    });
    let seconds = match value {
        serde_json::Value::Number(number) => number.as_f64()?,
        serde_json::Value::String(text) => {
            let text = text.trim();
            if !DIGITS.is_match(text) {
                return None;
            }
            text.parse().ok()?
        }
        _ => return None,
    };
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

/// How long to wait between polls of a device login. A value that states no
/// usable duration means the server has no preference, which is the
/// interval RFC 8628 § 3.5 names, 5 seconds, rather than a failed login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollInterval(pub Duration);

impl PollInterval {
    /// The interval without a server preference.
    pub const DEFAULT: Duration = Duration::from_secs(5);
}

impl Default for PollInterval {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl<'de> Deserialize<'de> for PollInterval {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let interval = seconds(&value).map_or(Self::DEFAULT, Duration::from_secs_f64);
        Ok(Self(interval))
    }
}

/// A device code's or a token's lifetime. A value that states no usable
/// duration, or none above zero, reads as absent. Use it with
/// `#[serde(default)]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lifetime(pub Option<Duration>);

impl<'de> Deserialize<'de> for Lifetime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let lifetime = seconds(&value)
            .filter(|seconds| *seconds > 0.0)
            .map(Duration::from_secs_f64);
        Ok(Self(lifetime))
    }
}

/// The error codes a token endpoint's refusal states (RFC 6749 § 5.2), and
/// those of a device login's poll (RFC 8628 § 3.5).
const OAUTH_ERRORS: [&str; 12] = [
    "invalid_request",
    "invalid_client",
    "invalid_grant",
    "unauthorized_client",
    "unsupported_grant_type",
    "invalid_scope",
    "server_error",
    "temporarily_unavailable",
    "authorization_pending",
    "slow_down",
    "access_denied",
    "expired_token",
];

/// A token endpoint's refusal (RFC 6749 § 5.2): the `error` code of a body that
/// is a JSON object, when it is one OAuth defines.
///
/// Nothing else of the body is kept, its `error_description` included, so no
/// log or message repeats what a vendor wrote, which may quote a token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenRefusal {
    /// The OAuth code; none for a body that is no JSON object, states no
    /// code or one OAuth does not define.
    code: Option<&'static str>,
}

/// The fields of a refusal's JSON object that are read.
#[derive(Deserialize)]
struct RefusalObject {
    #[serde(default)]
    error: ReportedString,
}

impl TokenRefusal {
    /// The refusal in `body`.
    pub fn of(body: &[u8]) -> Self {
        // A body that is not a JSON object states no code: serde would read
        // the struct from an array too.
        let Ok(object) = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(body)
        else {
            return Self::default();
        };
        let Ok(refusal) = serde_json::from_value::<RefusalObject>(object.into()) else {
            return Self::default();
        };
        let code = refusal
            .error
            .as_deref()
            .and_then(|error| OAUTH_ERRORS.into_iter().find(|code| *code == error));
        Self { code }
    }

    /// The OAuth code the body states, if any.
    pub fn code(&self) -> Option<&'static str> {
        self.code
    }

    /// The code as logs and messages name it: OAuth's, or `other`.
    pub fn named(&self) -> &'static str {
        self.code.unwrap_or("other")
    }

    /// Whether the body states `code`.
    pub fn is(&self, code: &str) -> bool {
        self.code == Some(code)
    }
}

/// Why a login or refresh response could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResponseError {
    /// The body broke off.
    #[error("the response body could not be read")]
    Unreadable,
    /// The body is not what the step answers; the error never quotes it.
    #[error("the response is {0}")]
    Malformed(SecretDecodeError),
}

/// Decodes the JSON body of a login or refresh response into `T`. A failure
/// names the field's path and its kind only, because the body holds tokens.
pub async fn decode_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ResponseError> {
    let Ok(text) = response.text().await else {
        return Err(ResponseError::Unreadable);
    };
    decode_secret(&text).map_err(ResponseError::Malformed)
}

/// The base64url alphabet with or without padding, as tokens use it.
const BASE64_URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// The claims of a JWT, read without verifying its signature.
///
/// The vendor that receives the token decides whether it is good, and Demi
/// reads only the account's identity and the token's expiry. `None` when the
/// token is not three segments, its payload is not base64url JSON, or the
/// claims are not a `C`. `C` reads a claim of the wrong type as absent where a
/// missing claim is no reason to refuse the token.
pub fn jwt_claims<C: DeserializeOwned>(token: &str) -> Option<C> {
    let mut segments = token.split('.');
    let (Some(_header), Some(payload), Some(_signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return None;
    };
    if payload.is_empty() {
        return None;
    }
    let bytes = BASE64_URL.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}
