//! Browser sessions (`backend.md` § Authentication and ownership): the
//! cookie holds a random 256-bit token, storage holds the token's SHA-256,
//! and a session expires 30 days after its last renewal.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::UserId;
use jiff::SignedDuration;

use demi_backend_database::StorageError;
use demi_backend_database::accounts::{ResolvedSession, SessionPolicy, TokenHash};
use demi_backend_database::control::ControlService;

const DAY_SECONDS: i64 = 24 * 60 * 60;

const SESSION_POLICY: SessionPolicy = SessionPolicy {
    lifetime: SignedDuration::from_secs(30 * DAY_SECONDS),
    renew_below: SignedDuration::from_secs(15 * DAY_SECONDS),
};

/// The policy of a look that never renews: a synchronization channel's,
/// which only requests renew (`backend.md` § Page synchronization).
const WITHOUT_RENEWAL: SessionPolicy = SessionPolicy {
    renew_below: SignedDuration::ZERO,
    ..SESSION_POLICY
};

/// A new session: the token for the cookie and when the session expires.
pub struct OpenedSession {
    pub token: String,
    pub expires_at: Timestamp,
}

/// Opens, resolves and closes sessions in the control database.
pub struct WebSessions {
    control: ControlService,
}

impl WebSessions {
    pub fn new(control: ControlService) -> Self {
        Self { control }
    }

    pub async fn open(&self, user: UserId) -> Result<OpenedSession, StorageError> {
        let token = hex::encode(rand::random::<[u8; 32]>());
        let expires_at = self
            .control
            .open_web_session(TokenHash::of(&token), user, SESSION_POLICY)
            .await?;
        Ok(OpenedSession { token, expires_at })
    }

    /// The session a cookie's token names, or `None` when it names no live
    /// session.
    pub async fn resolve(&self, token: &str) -> Result<Option<ResolvedSession>, StorageError> {
        self.control
            .resolve_web_session(TokenHash::of(token), SESSION_POLICY)
            .await
    }

    /// The live session a token's hash names, as `resolve` finds it but
    /// never renewed; `None` when it names no live session.
    pub async fn check(&self, token: &TokenHash) -> Result<Option<ResolvedSession>, StorageError> {
        self.control.resolve_web_session(token.clone(), WITHOUT_RENEWAL).await
    }

    pub async fn close(&self, token: &str) -> Result<(), StorageError> {
        self.control.close_web_session(TokenHash::of(token)).await
    }
}
