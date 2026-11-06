//! What storage keeps of a token instead of the token.

use sha2::{Digest, Sha256};

/// The SHA-256 of a session or device token, or of a handoff code, in
/// lowercase hexadecimal: what storage keeps instead of the token
/// (`crates/backend/src/config/secret.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHash(String);

impl TokenHash {
    pub fn of(token: &str) -> Self {
        Self(hex::encode(Sha256::digest(token.as_bytes())))
    }

    /// A hash as storage kept it; only a row of a table of hashes holds one.
    pub fn from_stored(hash: String) -> Self {
        Self(hash)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
