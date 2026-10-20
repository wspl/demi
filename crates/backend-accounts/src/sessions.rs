//! The web app's login sessions: the
//! cookie holds a random 256-bit token, storage holds the token's SHA-256,
//! and a session expires 30 days after its last renewal. Each is named for
//! the browser and system it signed in from, and a request writes its last
//! activity at most once an hour. A session verified by password or passkey
//! in the last five minutes makes handoff codes and adds passkeys; a sign-in
//! counts as a verification, a handoff code's too, which a phone keeping its
//! session is verified by. Handoff codes are 256 random bits, kept as their
//! SHA-256 for ten minutes and spent once; they open the account on a phone:
//! in the phone's own session of the account when it has one, else in a new
//! session.
//!
//! A handoff code travels after `#` in its URL, which a browser never sends
//! to a server, so no access log or proxy holds it. Its `to` is a path of
//! the product, checked when the code is made and again when it is spent, so
//! a code never sends a phone to another site. The laptop learns of the
//! spending from the account's sessions, the synchronization channel's
//! `sessions` part, whose new session names the code that made it; it never
//! asks. Signing a session out deletes it and closes its channel and its
//! conversation sockets with `session_ended`, and so does an expiry.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::sessions::{HandoffCode, HandoffId, ProductPath, SessionId, SessionList};
use jiff::SignedDuration;
use url::Url;

use demi_backend_database::StorageError;
use demi_backend_database::accounts::{ResolvedSession, SessionPolicy, TokenHash};
use demi_backend_database::control::ControlService;
use demi_backend_database::web_sessions::{HandoffIssue, NewSession};

use crate::user_agent::browser_and_system;

const DAY_SECONDS: i64 = 24 * 60 * 60;

const SESSION_POLICY: SessionPolicy = SessionPolicy {
    lifetime: SignedDuration::from_secs(30 * DAY_SECONDS),
    renew_below: SignedDuration::from_secs(15 * DAY_SECONDS),
    activity_every: Some(SignedDuration::from_hours(1)),
};

/// The policy of a look that never renews and is no activity: a
/// synchronization channel's, which only requests renew
/// (`crates/backend-user-shard/src/sync/mod.rs`).
const WITHOUT_RENEWAL: SessionPolicy = SessionPolicy {
    renew_below: SignedDuration::ZERO,
    activity_every: None,
    ..SESSION_POLICY
};

/// How long a verification lets the session make handoff codes.
const VERIFICATION_HOLDS: SignedDuration = SignedDuration::from_mins(5);

/// How long a handoff code can be spent.
const HANDOFF_LIFETIME: SignedDuration = SignedDuration::from_mins(10);

/// The name of a session whose User-Agent names no browser or system.
const UNNAMED_BROWSER: &str = "Web Browser";

/// A new session: the token for the cookie and when the session expires.
pub struct OpenedSession {
    pub token: String,
    pub expires_at: Timestamp,
}

/// How spending a handoff or sign-in code went.
pub enum Spend {
    /// The code is unknown, expired or spent.
    Unknown,
    /// The code's account is disabled, which refuses every sign-in.
    Disabled,
    Opened(HandedOff),
}

/// A handoff code just spent: the phone's session, its account, and the
/// page it opens.
pub struct HandedOff {
    pub session: PhoneSession,
    pub user: UserId,
    pub to: ProductPath,
}

/// The session a handoff code opens its account in on the phone.
pub enum PhoneSession {
    /// The phone's own session of the account, which now carries the code.
    Kept,
    /// A new session, made by the code; `ended` is the session of another
    /// account the phone held, which it replaces.
    Opened {
        session: OpenedSession,
        ended: Option<EndedSession>,
    },
}

/// A session that ended, by its account and its token's hash, whose pages'
/// channels end with it.
pub struct EndedSession {
    pub user: UserId,
    pub token: TokenHash,
}

/// How signing out another session went.
pub enum SignOut {
    /// The session is gone; its pages' channels end with it.
    Done(TokenHash),
    /// It is the caller's own session, which Sign Out ends.
    Current,
    /// The account has no session of that id.
    NotFound,
}

/// Opens, resolves and closes sessions in the control database, and makes
/// and spends handoff codes.
pub struct WebSessions {
    control: ControlService,
}

impl WebSessions {
    pub fn new(control: ControlService) -> Self {
        Self { control }
    }

    /// Signs `user` in from the browser `user_agent` names.
    pub async fn open(
        &self,
        user: UserId,
        user_agent: Option<&str>,
    ) -> Result<OpenedSession, StorageError> {
        self.open_made_by(user, user_agent, None).await
    }

    async fn open_made_by(
        &self,
        user: UserId,
        user_agent: Option<&str>,
        handoff: Option<(HandoffId, Option<UserId>)>,
    ) -> Result<OpenedSession, StorageError> {
        let token = hex::encode(rand::random::<[u8; 32]>());
        let name = user_agent
            .and_then(browser_and_system)
            .unwrap_or_else(|| UNNAMED_BROWSER.to_owned());
        let session = NewSession {
            token: TokenHash::of(&token),
            user,
            name,
            handoff,
        };
        let expires_at = self.control.open_web_session(session, SESSION_POLICY).await?;
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
        self.control
            .resolve_web_session(token.clone(), WITHOUT_RENEWAL)
            .await
    }

    pub async fn close(&self, token: &str) -> Result<(), StorageError> {
        self.control.close_web_session(TokenHash::of(token)).await
    }

    /// Marks the session verified now: its user proved it was them on it.
    pub async fn verify(&self, token: &TokenHash) -> Result<(), StorageError> {
        self.control.verify_web_session(token.clone()).await
    }

    /// Whether a session last verified at `verified_at` makes handoff codes
    /// now.
    pub fn verification_holds(&self, verified_at: Option<Timestamp>) -> bool {
        verified_at.is_some_and(|at| {
            self.control.now().to_jiff().duration_since(at.to_jiff()) < VERIFICATION_HOLDS
        })
    }

    /// `user`'s sessions as the session `reader` lists them, newest first.
    pub async fn list(&self, user: &UserId, reader: &TokenHash) -> Result<SessionList, StorageError> {
        let sessions = self.control.web_sessions(user.clone()).await?;
        Ok(SessionList {
            sessions: sessions.iter().map(|session| session.dto(reader)).collect(),
        })
    }

    /// Signs `user`'s session `id` out for the caller, whose own session is
    /// `current`.
    pub async fn sign_out(
        &self,
        user: &UserId,
        id: SessionId,
        current: &SessionId,
    ) -> Result<SignOut, StorageError> {
        if id == *current {
            return Ok(SignOut::Current);
        }
        Ok(match self.control.sign_out_web_session(user.clone(), id).await? {
            Some(token) => SignOut::Done(token),
            None => SignOut::NotFound,
        })
    }

    /// A new handoff code of `user`, which opens `to` at the backend's
    /// public URL `public`, in place of the user's previous one; with
    /// `issued_by`, a sign-in code the administrator makes for the account,
    /// in place of its previous sign-in code. The caller checked the
    /// session's verification.
    pub async fn make_handoff(
        &self,
        user: &UserId,
        issued_by: Option<&UserId>,
        to: ProductPath,
        public: &Url,
    ) -> Result<HandoffCode, StorageError> {
        let code = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let id = HandoffId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        let issue = HandoffIssue {
            id: id.clone(),
            user: user.clone(),
            code: TokenHash::of(&code),
            to,
            issued_by: issued_by.cloned(),
        };
        self.control.issue_handoff_code(issue, HANDOFF_LIFETIME).await?;
        let mut url = public.join("/handoff").expect("a path joins any HTTP URL");
        url.set_fragment(Some(&code));
        Ok(HandoffCode {
            id,
            url: url.into(),
            expires_in_ms: HANDOFF_LIFETIME
                .as_millis()
                .try_into()
                .expect("ten minutes are a positive count of milliseconds"),
        })
    }

    /// Spends the handoff `code` from the browser `user_agent` names, whose
    /// cookie holds the session token `current`, if any: a live session of
    /// the code's account is kept and carries the code; any other is ended
    /// and the account signed in anew. A code of a disabled account is
    /// spent and signs nobody in.
    pub async fn spend_handoff(
        &self,
        code: &str,
        current: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<Spend, StorageError> {
        let Some(spent) = self.control.spend_handoff_code(TokenHash::of(code)).await? else {
            return Ok(Spend::Unknown);
        };
        let disabled = self
            .control
            .account(spent.user.clone())
            .await?
            .is_none_or(|account| account.user.disabled);
        if disabled {
            return Ok(Spend::Disabled);
        }
        let mut ended = None;
        if let Some(token) = current.map(TokenHash::of) {
            match self.check(&token).await? {
                Some(held) if held.user.id == spent.user => {
                    self.control
                        .record_handoff_on_web_session(token, spent.id)
                        .await?;
                    return Ok(Spend::Opened(HandedOff {
                        session: PhoneSession::Kept,
                        user: spent.user,
                        to: spent.to,
                    }));
                }
                Some(held) => {
                    self.control.close_web_session(token.clone()).await?;
                    ended = Some(EndedSession {
                        user: held.user.id,
                        token,
                    });
                }
                None => {}
            }
        }
        let session = self
            .open_made_by(spent.user.clone(), user_agent, Some((spent.id, spent.issued_by)))
            .await?;
        Ok(Spend::Opened(HandedOff {
            session: PhoneSession::Opened { session, ended },
            user: spent.user,
            to: spent.to,
        }))
    }
}
