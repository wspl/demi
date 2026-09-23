//! Values the runner's records and its wire share, each checked where it is
//! read, so code past the reading never checks them again.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// A value that breaks its type's rule.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ValueError(String);

/// A backend's URL (`runner.md` § Connection and identity): `http`, `https`,
/// `ws` or `wss`, naming a host, without credentials or a fragment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BackendUrl(url::Url);

impl BackendUrl {
    /// The URL in its normal form, which names the installation.
    pub fn url(&self) -> &url::Url {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<String> for BackendUrl {
    type Error = ValueError;

    fn try_from(value: String) -> Result<Self, ValueError> {
        value.parse()
    }
}

impl FromStr for BackendUrl {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, ValueError> {
        let url = url::Url::parse(value).map_err(|error| ValueError(error.to_string()))?;
        if !matches!(url.scheme(), "http" | "https" | "ws" | "wss")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(ValueError("invalid backend URL".into()));
        }
        Ok(Self(url))
    }
}

impl From<BackendUrl> for String {
    fn from(url: BackendUrl) -> Self {
        url.0.into()
    }
}

impl fmt::Display for BackendUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A device's credential: 1 to 4096 characters, none of them whitespace. It
/// never appears in debugging output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeviceToken(String);

impl DeviceToken {
    /// The credential itself, for the header or file that carries it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DeviceToken {
    type Error = ValueError;

    fn try_from(value: String) -> Result<Self, ValueError> {
        let length = value.encode_utf16().count();
        if length == 0 || length > 4096 || value.chars().any(char::is_whitespace) {
            return Err(ValueError("invalid device token".into()));
        }
        Ok(Self(value))
    }
}

impl FromStr for DeviceToken {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, ValueError> {
        value.to_owned().try_into()
    }
}

impl From<DeviceToken> for String {
    fn from(token: DeviceToken) -> Self {
        token.0
    }
}

impl fmt::Debug for DeviceToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeviceToken(..)")
    }
}
