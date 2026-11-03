//! A share key as a person copies it: one line that names the lending server
//! and holds the token it checks, `demi-share-key-` and the unpadded
//! base64url of `{"url": "https://demi.example.com/", "token": "…"}`.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use url::Url;

const PREFIX: &str = "demi-share-key-";

/// A share key's parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShareKey {
    /// The lending server's public URL.
    pub url: Url,
    /// What the lender keeps the SHA-256 of.
    pub token: String,
}

/// Why pasted text is not a share key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("This is not a share key. A share key starts with \"{PREFIX}\".")]
pub struct NotAKey;

impl ShareKey {
    /// The key as a person copies it.
    pub fn text(&self) -> String {
        let json = serde_json::to_vec(self).expect("a share key serializes");
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
    }

    /// The key `text` holds, with the white space around it that a copy
    /// brings left out.
    pub fn parse(text: &str) -> Result<Self, NotAKey> {
        let encoded = text.trim().strip_prefix(PREFIX).ok_or(NotAKey)?;
        let json = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| NotAKey)?;
        let key: Self = serde_json::from_slice(&json).map_err(|_| NotAKey)?;
        if key.token.is_empty() || !matches!(key.url.scheme(), "http" | "https") {
            return Err(NotAKey);
        }
        Ok(key)
    }
}
