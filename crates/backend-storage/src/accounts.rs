//! The account records of the control store (`storage.md` § Control
//! records, § Passwords and credentials at rest): an account with its
//! password hash, the hashes that stand for session and device tokens and
//! for email-change codes, and the policies and outcomes of sessions and
//! challenges. The accounts service decides the policies; storage keeps the
//! records they govern.

use std::fmt;

use demi_core::Timestamp;
use demi_web_api::auth::UserDto;
use demi_web_api::ids::UserId;
use demi_web_api::text::EmailAddress;
use hmac::{Hmac, Mac};
use jiff::SignedDuration;
use password_hash::PasswordHash as PhcString;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// An account with the hash its password checks against.
pub struct Account {
    pub user: UserDto,
    pub password_hash: PasswordHash,
}

/// A stored password hash: a PHC string such as
/// `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`, which records its
/// algorithm and parameters beside the salt and the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    /// A PHC string read back from storage.
    pub fn parse(text: String) -> Result<Self, password_hash::Error> {
        PhcString::new(&text)?;
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&PhcString<'_>> for PasswordHash {
    fn from(hash: &PhcString<'_>) -> Self {
        Self(hash.to_string())
    }
}

impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordHash(..)")
    }
}

/// The SHA-256 of a session or device token in lowercase hexadecimal: what
/// storage keeps instead of the token (`storage.md` § Passwords and
/// credentials at rest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHash(String);

impl TokenHash {
    pub fn of(token: &str) -> Self {
        Self(hex::encode(Sha256::digest(token.as_bytes())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How long a session lives after its last renewal, and how little of that
/// must remain for a request to renew it.
#[derive(Debug, Clone, Copy)]
pub struct SessionPolicy {
    pub lifetime: SignedDuration,
    pub renew_below: SignedDuration,
}

/// A live session's user and expiry, and whether this request renewed it.
pub struct ResolvedSession {
    pub user: UserDto,
    pub expires_at: Timestamp,
    pub renewed: bool,
}

/// How long a code lasts, how soon another may be sent, and how many wrong
/// codes a challenge takes.
#[derive(Debug, Clone, Copy)]
pub struct ChallengePolicy {
    pub lifetime: SignedDuration,
    pub cooldown: SignedDuration,
    pub attempts: u32,
}

/// The HMAC-SHA256 of a challenge's id and code in lowercase hexadecimal,
/// under the email-change key: what storage keeps instead of the code.
#[derive(Debug, Clone)]
pub struct CodeHash(String);

impl CodeHash {
    /// The hash of `code`, sent for `challenge`, under the email-change key
    /// `key`.
    pub fn of(key: &[u8], challenge: &str, code: &str) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
        mac.update(format!("email-change:{challenge}:{code}").as_bytes());
        Self(hex::encode(mac.finalize().into_bytes()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Compares in constant time, so the comparison does not reveal how much
    /// of a stored hash a guess matched.
    pub fn matches(&self, stored: &str) -> bool {
        self.0.as_bytes().ct_eq(stored.as_bytes()).into()
    }
}

/// A challenge to store: the new address, and the password hash it was
/// issued under, so a password change ends it.
pub struct ChallengeIssue {
    pub user: UserId,
    pub id: String,
    pub email: EmailAddress,
    pub password_hash: PasswordHash,
    pub code_hash: CodeHash,
}

/// How a confirmation ended.
pub enum ChallengeOutcome {
    /// The account has the new address; the challenge is consumed.
    Changed(UserDto),
    /// The code is wrong, expired or used up, or the challenge no longer
    /// holds.
    InvalidCode,
    /// Another account took the address after the challenge was issued.
    EmailTaken,
}
