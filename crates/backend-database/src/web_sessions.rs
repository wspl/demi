//! The web app's sessions and handoff codes (`storage.md` § Control
//! records, `backend.md` § Authentication and ownership): each session by
//! the hash of its token, with the browser it signed in from, when it signed
//! in, was last active and was last verified, and the handoff code that made
//! it; each user's one handoff code by the hash of the code. The accounts
//! service decides the policies; storage keeps the records they govern.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::sessions::{HandoffId, ProductPath, SessionDto, SessionId};
use jiff::SignedDuration;
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::accounts::{ResolvedSession, SessionPolicy, TokenHash};
use super::columns::{decode, instant};
use super::control::{ControlService, later, user_row};

const SESSIONS: &str = "web_sessions";
const HANDOFFS: &str = "handoff_codes";
const COLUMNS: &str = "token_hash, id, name, created_at, last_active_at, handoff_id";

/// A session to open: its token's hash, its user, the browser and system it
/// signs in from, and the handoff code that makes it, if one does.
pub struct NewSession {
    pub token: TokenHash,
    pub user: UserId,
    pub name: String,
    pub handoff: Option<HandoffId>,
}

/// A live session of an account, as the account's list shows it.
pub struct SessionRecord {
    pub token: TokenHash,
    pub id: SessionId,
    pub name: String,
    pub created_at: Timestamp,
    pub last_active_at: Timestamp,
    pub handoff: Option<HandoffId>,
}

impl SessionRecord {
    /// The session as the list of the session `reader` shows it.
    pub fn dto(&self, reader: &TokenHash) -> SessionDto {
        SessionDto {
            id: self.id.clone(),
            name: self.name.clone(),
            created_at: self.created_at,
            last_active_at: self.last_active_at,
            current: self.token == *reader,
            handoff_id: self.handoff.clone(),
        }
    }
}

/// A handoff code to keep in place of the user's previous one: its id, the
/// hash of the code, and the page it opens.
pub struct HandoffIssue {
    pub id: HandoffId,
    pub user: UserId,
    pub code: TokenHash,
    pub to: ProductPath,
}

/// A handoff code just spent: its id, its user and the page it opens.
pub struct SpentHandoff {
    pub id: HandoffId,
    pub user: UserId,
    pub to: ProductPath,
}

impl ControlService {
    /// Stores a new session, signed in, active and verified now, since a
    /// sign-in proves it is the user, and answers when it expires. A sign-in is the one moment the table grows, so it also
    /// drops the sessions that ran out unnoticed.
    pub async fn open_web_session(
        &self,
        session: NewSession,
        policy: SessionPolicy,
    ) -> Result<Timestamp, StorageError> {
        let id = SessionId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let expires_at = later(now, policy.lifetime)?;
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM web_sessions WHERE expires_at <= ?1",
                [now.as_millisecond()],
            )?;
            transaction.execute(
                "INSERT INTO web_sessions
                   (token_hash, id, user_id, expires_at, name, created_at, last_active_at, verified_at, handoff_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?6, ?7)",
                params![
                    session.token.as_str(),
                    id.as_str(),
                    session.user.as_str(),
                    expires_at.as_millisecond(),
                    session.name,
                    now.as_millisecond(),
                    session.handoff.as_ref().map(HandoffId::as_str),
                ],
            )?;
            transaction.commit()?;
            Ok(expires_at)
        })
        .await
    }

    /// The live session a token hash names, renewed when less than the
    /// policy's margin remains, and its last activity written when the
    /// policy's interval has passed since; an expired session is deleted and
    /// is `None`.
    pub async fn resolve_web_session(
        &self,
        token: TokenHash,
        policy: SessionPolicy,
    ) -> Result<Option<ResolvedSession>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let found = {
                let mut statement = transaction.prepare_cached(
                    "SELECT s.id AS session_id, s.expires_at, s.last_active_at, s.verified_at,
                            u.id, u.email, u.nickname, u.role, u.created_at
                     FROM web_sessions s JOIN users u ON u.id = s.user_id
                     WHERE s.token_hash = ?1",
                )?;
                let mut rows = statement.query([token.as_str()])?;
                match rows.next()? {
                    Some(row) => Some((
                        decode(SESSIONS, "id", SessionId::try_from(row.get::<_, String>("session_id")?))?,
                        instant(row, SESSIONS, "expires_at")?,
                        instant(row, SESSIONS, "last_active_at")?,
                        optional_instant(row, "verified_at")?,
                        user_row(row)?,
                    )),
                    None => None,
                }
            };
            let Some((id, expires_at, last_active_at, verified_at, user)) = found else {
                return Ok(None);
            };
            if expires_at <= now {
                transaction.execute(
                    "DELETE FROM web_sessions WHERE token_hash = ?1",
                    [token.as_str()],
                )?;
                transaction.commit()?;
                return Ok(None);
            }
            let renewed = expires_at.to_jiff().duration_since(now.to_jiff()) < policy.renew_below;
            let expires_at = if renewed {
                let extended = later(now, policy.lifetime)?;
                transaction.execute(
                    "UPDATE web_sessions SET expires_at = ?1 WHERE token_hash = ?2",
                    params![extended.as_millisecond(), token.as_str()],
                )?;
                extended
            } else {
                expires_at
            };
            let active = policy
                .activity_every
                .is_some_and(|every| now.to_jiff().duration_since(last_active_at.to_jiff()) >= every);
            if active {
                transaction.execute(
                    "UPDATE web_sessions SET last_active_at = ?1 WHERE token_hash = ?2",
                    params![now.as_millisecond(), token.as_str()],
                )?;
            }
            transaction.commit()?;
            Ok(Some(ResolvedSession {
                id,
                user,
                expires_at,
                verified_at,
                renewed,
                active,
            }))
        })
        .await
    }

    pub async fn close_web_session(&self, token: TokenHash) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM web_sessions WHERE token_hash = ?1",
                [token.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Marks the session `token` names verified now.
    pub async fn verify_web_session(&self, token: TokenHash) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                "UPDATE web_sessions SET verified_at = ?1 WHERE token_hash = ?2",
                params![now.as_millisecond(), token.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The user's live sessions, newest first.
    pub async fn web_sessions(&self, user: UserId) -> Result<Vec<SessionRecord>, StorageError> {
        self.call(move |connection, now| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM web_sessions WHERE user_id = ?1 AND expires_at > ?2
                 ORDER BY created_at DESC, rowid DESC"
            ))?;
            let mut rows = statement.query(params![user.as_str(), now.as_millisecond()])?;
            let mut sessions = Vec::new();
            while let Some(row) = rows.next()? {
                sessions.push(session_row(row)?);
            }
            Ok(sessions)
        })
        .await
    }

    /// Deletes the user's session `id` and answers its token's hash; `None`
    /// when the user has no such session.
    pub async fn sign_out_web_session(
        &self,
        user: UserId,
        id: SessionId,
    ) -> Result<Option<TokenHash>, StorageError> {
        self.call(move |connection, _| {
            let token: Option<String> = connection
                .query_row(
                    "DELETE FROM web_sessions WHERE user_id = ?1 AND id = ?2 RETURNING token_hash",
                    params![user.as_str(), id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(token.map(TokenHash::from_stored))
        })
        .await
    }

    /// Keeps `issue` as its user's one handoff code, ending the previous
    /// one, until `lifetime` from now.
    pub async fn issue_handoff_code(
        &self,
        issue: HandoffIssue,
        lifetime: SignedDuration,
    ) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            let expires_at = later(now, lifetime)?;
            connection.execute(
                "INSERT INTO handoff_codes (user_id, id, code_hash, to_path, expires_at, spent)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0)
                 ON CONFLICT (user_id) DO UPDATE SET
                   id = excluded.id, code_hash = excluded.code_hash, to_path = excluded.to_path,
                   expires_at = excluded.expires_at, spent = 0",
                params![
                    issue.user.as_str(),
                    issue.id.as_str(),
                    issue.code.as_str(),
                    issue.to.as_str(),
                    expires_at.as_millisecond()
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Spends the handoff code whose hash is `code` once; `None` when no
    /// live, unspent code has it.
    pub async fn spend_handoff_code(
        &self,
        code: TokenHash,
    ) -> Result<Option<SpentHandoff>, StorageError> {
        self.call(move |connection, now| {
            let mut statement = connection.prepare_cached(
                "UPDATE handoff_codes SET spent = 1
                 WHERE code_hash = ?1 AND spent = 0 AND expires_at > ?2
                 RETURNING id, user_id, to_path",
            )?;
            let mut rows = statement.query(params![code.as_str(), now.as_millisecond()])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(SpentHandoff {
                id: decode(HANDOFFS, "id", HandoffId::try_from(row.get::<_, String>("id")?))?,
                user: decode(HANDOFFS, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
                // A path checked as the code was made is checked again as it
                // is spent.
                to: decode(HANDOFFS, "to_path", ProductPath::try_from(row.get::<_, String>("to_path")?))?,
            }))
        })
        .await
    }
}

fn session_row(row: &Row<'_>) -> Result<SessionRecord, StorageError> {
    Ok(SessionRecord {
        token: TokenHash::from_stored(row.get("token_hash")?),
        id: decode(SESSIONS, "id", SessionId::try_from(row.get::<_, String>("id")?))?,
        name: row.get("name")?,
        created_at: instant(row, SESSIONS, "created_at")?,
        last_active_at: instant(row, SESSIONS, "last_active_at")?,
        handoff: row
            .get::<_, Option<String>>("handoff_id")?
            .map(|id| decode(SESSIONS, "handoff_id", HandoffId::try_from(id)))
            .transpose()?,
    })
}

fn optional_instant(row: &Row<'_>, column: &'static str) -> Result<Option<Timestamp>, StorageError> {
    row.get::<_, Option<i64>>(column)?
        .map(|millisecond| decode(SESSIONS, column, Timestamp::from_millisecond(millisecond)))
        .transpose()
}
