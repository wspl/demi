//! What a plugin keeps and sends beyond its values: the names of its secrets, and one
//! outbound HTTPS request with its answer and the ways it can fail.

use std::fmt;
use std::time::Duration;

use demi_shared_types::B64Bytes;
use serde::{Deserialize, Serialize};
use url::Url;

/// The name of a plugin's secret, such as `api-key`: 1 to 64 lowercase
/// letters, digits, hyphens and underscores. The plugin host keeps the
/// secrets of a plugin's OAuth sign-in under names outside this rule, so a
/// plugin never reads them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretName(String);

/// Why a text is not a secret's name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("\"{0}\" is not a secret's name: 1 to 64 lowercase letters, digits, hyphens and underscores")]
pub struct SecretNameError(String);

impl SecretName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SecretName {
    type Error = SecretNameError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        let valid = (1..=64).contains(&name.len())
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            });
        if valid {
            Ok(Self(name))
        } else {
            Err(SecretNameError(name))
        }
    }
}

impl TryFrom<&str> for SecretName {
    type Error = SecretNameError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        Self::try_from(name.to_owned())
    }
}

impl From<SecretName> for String {
    fn from(name: SecretName) -> Self {
        name.0
    }
}

impl fmt::Display for SecretName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A secret and the revision a write of it names. `Debug` never shows its
/// text.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredSecret {
    pub value: String,
    pub revision: u64,
}

impl fmt::Debug for StoredSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredSecret")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// One outbound request (`crates/backend-plugins/src/web.rs`): only `https`,
/// with a deadline and a limit on the response's body the plugin names.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebRequest {
    pub method: WebMethod,
    pub url: Url,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<WebHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<B64Bytes>,
    /// How long the request may take, the response's body included, in
    /// milliseconds: 1 to [`WebRequest::MAX_DEADLINE`].
    pub deadline_ms: u64,
    /// The most bytes of the response's body: 1 to
    /// [`WebRequest::MAX_BODY_BYTES`].
    pub limit_bytes: u64,
}

impl WebRequest {
    /// The longest deadline a plugin may name.
    pub const MAX_DEADLINE: Duration = Duration::from_secs(60);
    /// The largest response body a plugin may accept.
    pub const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

    /// A `method` request of `url`, with no header and no body, a deadline
    /// of `deadline` and a body limit of `limit_bytes`.
    pub fn new(method: WebMethod, url: Url, deadline: Duration, limit_bytes: u64) -> Self {
        Self {
            method,
            url,
            headers: Vec::new(),
            body: None,
            deadline_ms: u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
            limit_bytes,
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push(WebHeader {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(B64Bytes::new(body.into()));
        self
    }

    pub fn deadline(&self) -> Duration {
        Duration::from_millis(self.deadline_ms)
    }
}

/// A request's headers and body may hold a key or a token, so `Debug`
/// shows only its method and URL.
impl fmt::Debug for WebRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebRequest")
            .field("method", &self.method)
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum WebMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

/// A header of a request or a response.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebHeader {
    pub name: String,
    pub value: String,
}

impl fmt::Debug for WebHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "WebHeader({}: ..)", self.name)
    }
}

/// The answer to a [`WebRequest`]: any status, as the vendor answered.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebResponse {
    pub status: u16,
    pub headers: Vec<WebHeader>,
    pub body: B64Bytes,
}

impl WebResponse {
    /// The value of the first header `name`, compared without case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }
}

impl fmt::Debug for WebResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebResponse")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// Why an outbound request, or a step of an OAuth sign-in, did not answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "reason",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WebFailure {
    /// The URL is not `https`, or a redirect led away from it.
    #[error("only https URLs are sent")]
    NotHttps,
    /// The request breaks a rule of its own, such as a deadline over 60
    /// seconds or a header that is not one.
    #[error("{message}")]
    Invalid { message: String },
    /// No answer came within the request's deadline.
    #[error("the request took longer than its deadline")]
    TimedOut,
    /// The response's body is larger than the request's limit.
    #[error("the response's body is larger than the request's limit")]
    TooLarge,
    /// The vendor could not be reached, or the connection broke.
    #[error("{message}")]
    Unreachable { message: String },
    /// The vendor's authorization server answered what an OAuth step cannot
    /// use, such as a registration it refused.
    #[error("{message}")]
    Vendor { message: String },
}
