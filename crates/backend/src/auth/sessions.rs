//! Browser sessions (`backend.md` § Authentication and ownership): the
//! cookie holds a random 256-bit token, storage holds the token's SHA-256,
//! and a session expires 30 days after its last renewal.

use demi_core::Timestamp;
use demi_web_api::auth::UserDto;
use demi_web_api::ids::UserId;
use jiff::SignedDuration;
use sha2::{Digest, Sha256};

use crate::storage::StorageError;
use crate::storage::control::ControlService;

/// How long a session lives after its last renewal, and how little of that
/// must remain for a request to renew it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionPolicy {
    pub(crate) lifetime: SignedDuration,
    pub(crate) renew_below: SignedDuration,
}

const DAY_SECONDS: i64 = 24 * 60 * 60;

pub(crate) const SESSION_POLICY: SessionPolicy = SessionPolicy {
    lifetime: SignedDuration::from_secs(30 * DAY_SECONDS),
    renew_below: SignedDuration::from_secs(15 * DAY_SECONDS),
};

/// The SHA-256 of a session or device token in lowercase hexadecimal: what
/// storage keeps instead of the token (`storage.md` § Passwords and
/// credentials at rest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TokenHash(String);

impl TokenHash {
    pub(crate) fn of(token: &str) -> Self {
        Self(hex::encode(Sha256::digest(token.as_bytes())))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A new session: the token for the cookie and when the session expires.
pub(crate) struct OpenedSession {
    pub(crate) token: String,
    pub(crate) expires_at: Timestamp,
}

/// A live session's user and expiry, and whether this request renewed it.
pub(crate) struct ResolvedSession {
    pub(crate) user: UserDto,
    pub(crate) expires_at: Timestamp,
    pub(crate) renewed: bool,
}

/// Opens, resolves and closes sessions in the control database.
pub(crate) struct WebSessions {
    control: ControlService,
}

impl WebSessions {
    pub(crate) fn new(control: ControlService) -> Self {
        Self { control }
    }

    pub(crate) async fn open(&self, user: UserId) -> Result<OpenedSession, StorageError> {
        let token = hex::encode(rand::random::<[u8; 32]>());
        let expires_at = self
            .control
            .open_web_session(TokenHash::of(&token), user, SESSION_POLICY)
            .await?;
        Ok(OpenedSession { token, expires_at })
    }

    /// The session a cookie's token names, or `None` when it names no live
    /// session.
    pub(crate) async fn resolve(&self, token: &str) -> Result<Option<ResolvedSession>, StorageError> {
        self.control
            .resolve_web_session(TokenHash::of(token), SESSION_POLICY)
            .await
    }

    pub(crate) async fn close(&self, token: &str) -> Result<(), StorageError> {
        self.control.close_web_session(TokenHash::of(token)).await
    }
}
