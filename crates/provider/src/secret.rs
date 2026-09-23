//! A credential's text as a provider holds it.

use std::fmt;

use http::HeaderValue;
use serde::{Deserialize, Serialize, Serializer};

/// A credential, such as an API key or a setup token: nonempty, one line of
/// text without control characters, and never printed. `Debug` shows
/// `Secret(..)`, so a logged configuration cannot leak it.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Secret(String);

/// Why a text is not a credential. It never holds the text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error("the credential is empty")]
    Empty,
    #[error("the credential contains a control character")]
    Control,
}

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The credential as a header value that HTTP logging hides.
    pub fn header_value(&self) -> HeaderValue {
        // A secret has no control character, which is what a header value
        // refuses.
        let mut value = HeaderValue::from_str(&self.0).expect("a secret is a header value");
        value.set_sensitive(true);
        value
    }
}

impl TryFrom<String> for Secret {
    type Error = SecretError;

    fn try_from(text: String) -> Result<Self, SecretError> {
        if text.is_empty() {
            return Err(SecretError::Empty);
        }
        if text.chars().any(char::is_control) {
            return Err(SecretError::Control);
        }
        Ok(Self(text))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(..)")
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
