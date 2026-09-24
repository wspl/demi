//! The control service (`storage.md` § Control records): every read and
//! write of `control.sqlite`, each one closure and one transaction on the
//! database's own thread. Operations take owned input, so no caller holds a
//! transaction open across a wait.

use std::path::Path;
use std::sync::Arc;

use demi_core::{Clock, Timestamp};
use demi_web_api::auth::{Role, UserDto};
use demi_web_api::ids::UserId;
use demi_web_api::settings::Preferences;
use demi_web_api::text::EmailAddress;
use jiff::SignedDuration;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::columns::{decode, instant, json, to_json};
use super::{StorageError, schema, sqlite};
use crate::auth::email_change::{ChallengeIssue, ChallengeOutcome, ChallengePolicy, CodeHash};
use crate::auth::passwords::PasswordHash;
use crate::auth::sessions::{ResolvedSession, SessionPolicy, TokenHash};
use crate::settings::{self, CheckedPatch};

/// An account with the hash its password checks against.
pub(crate) struct Account {
    pub(crate) user: UserDto,
    pub(crate) password_hash: PasswordHash,
}

#[derive(Clone)]
pub(crate) struct ControlService {
    db: tokio_rusqlite::Connection,
    clock: Arc<dyn Clock>,
}

const USER_COLUMNS: &str = "id, email, nickname, role, created_at";

impl ControlService {
    /// Opens the database, and gives a new one its schema.
    pub(crate) async fn open(path: &Path, clock: Arc<dyn Clock>) -> Result<Self, StorageError> {
        let db = tokio_rusqlite::Connection::open(path).await?;
        let control = Self { db, clock };
        control
            .call(|connection, _| {
                sqlite::configure(connection)?;
                schema::CONTROL.to_latest(connection)?;
                Ok(())
            })
            .await?;
        Ok(control)
    }

    /// Closes the database; operations after it fail with `Closed`.
    pub(crate) async fn close(&self) -> Result<(), StorageError> {
        sqlite::close(self.db.clone()).await
    }

    /// Runs `work` on the database thread with the time of the operation, in
    /// the whole milliseconds storage keeps.
    pub(super) async fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection, Timestamp) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, StorageError> {
        let clock = self.clock.clone();
        self.db
            .call(move |connection| work(connection, clock.now()))
            .await
            .map_err(sqlite::flatten)
    }

    pub(crate) async fn has_users(&self) -> Result<bool, StorageError> {
        self.call(|connection, _| {
            let found = connection.query_row("SELECT EXISTS (SELECT 1 FROM users)", [], |row| row.get(0))?;
            Ok(found)
        })
        .await
    }

    /// The instance's first account, created only while there is no account
    /// at all; `None` once setup has run.
    pub(crate) async fn create_master(
        &self,
        email: EmailAddress,
        password_hash: PasswordHash,
    ) -> Result<Option<UserDto>, StorageError> {
        let id = UserId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let set_up: bool =
                transaction.query_row("SELECT EXISTS (SELECT 1 FROM users)", [], |row| row.get(0))?;
            if set_up {
                return Ok(None);
            }
            transaction.execute(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                 VALUES (?1, ?2, '', ?3, ?4, ?5)",
                params![
                    id.as_str(),
                    email.as_str(),
                    password_hash.as_str(),
                    Role::Master.to_string(),
                    now.as_millisecond()
                ],
            )?;
            transaction.commit()?;
            Ok(Some(UserDto {
                id,
                email,
                nickname: String::new(),
                role: Role::Master,
                created_at: now,
            }))
        })
        .await
    }

    /// The login lookup.
    pub(crate) async fn account_by_email(&self, email: EmailAddress) -> Result<Option<Account>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {USER_COLUMNS}, password_hash FROM users WHERE email = ?1"
            ))?;
            let mut rows = statement.query([email.as_str()])?;
            rows.next()?.map(account).transpose()
        })
        .await
    }

    pub(crate) async fn account(&self, user: UserId) -> Result<Option<Account>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {USER_COLUMNS}, password_hash FROM users WHERE id = ?1"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            rows.next()?.map(account).transpose()
        })
        .await
    }

    /// Every account, in the order they were created.
    pub(crate) async fn users(&self) -> Result<Vec<UserDto>, StorageError> {
        self.call(|connection, _| {
            let mut statement =
                connection.prepare_cached(&format!("SELECT {USER_COLUMNS} FROM users ORDER BY created_at, rowid"))?;
            let mut rows = statement.query([])?;
            let mut users = Vec::new();
            while let Some(row) = rows.next()? {
                users.push(user_row(row)?);
            }
            Ok(users)
        })
        .await
    }

    /// A new account of `role`; `None`, writing nothing, when an account has
    /// the address already.
    pub(crate) async fn create_user(
        &self,
        email: EmailAddress,
        password_hash: PasswordHash,
        role: Role,
    ) -> Result<Option<UserDto>, StorageError> {
        let id = UserId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let created = connection.execute(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                 VALUES (?1, ?2, '', ?3, ?4, ?5) ON CONFLICT (email) DO NOTHING",
                params![
                    id.as_str(),
                    email.as_str(),
                    password_hash.as_str(),
                    role.to_string(),
                    now.as_millisecond()
                ],
            )?;
            Ok((created > 0).then(|| UserDto {
                id,
                email,
                nickname: String::new(),
                role,
                created_at: now,
            }))
        })
        .await
    }

    pub(crate) async fn email_in_use(&self, email: EmailAddress) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let found = connection.query_row(
                "SELECT EXISTS (SELECT 1 FROM users WHERE email = ?1)",
                [email.as_str()],
                |row| row.get(0),
            )?;
            Ok(found)
        })
        .await
    }

    /// Sets the nickname and answers the account as it now is.
    pub(crate) async fn set_nickname(&self, user: UserId, nickname: String) -> Result<Option<UserDto>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "UPDATE users SET nickname = ?1 WHERE id = ?2 RETURNING {USER_COLUMNS}"
            ))?;
            let mut rows = statement.query(params![nickname, user.as_str()])?;
            rows.next()?.map(user_row).transpose()
        })
        .await
    }

    pub(crate) async fn set_password(&self, user: UserId, password_hash: PasswordHash) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE users SET password_hash = ?1 WHERE id = ?2",
                params![password_hash.as_str(), user.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The user's saved preferences; a user who saved none has no overrides.
    pub(crate) async fn preferences(&self, user: UserId) -> Result<Preferences, StorageError> {
        self.call(move |connection, _| saved_preferences(connection, &user)).await
    }

    /// Merges `patch` into the user's preferences and answers them as saved.
    /// The read, the merge and the write are one transaction, so patches of
    /// different fields that arrive together all stay.
    pub(crate) async fn patch_preferences(&self, user: UserId, patch: CheckedPatch) -> Result<Preferences, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let preferences = settings::merge(saved_preferences(&transaction, &user)?, patch);
            transaction.execute(
                "INSERT INTO user_preferences (user_id, preferences) VALUES (?1, ?2)
                 ON CONFLICT (user_id) DO UPDATE SET preferences = excluded.preferences",
                params![user.as_str(), to_json(&preferences)],
            )?;
            transaction.commit()?;
            Ok(preferences)
        })
        .await
    }

    /// Stores a new session and answers when it expires. A login is the one
    /// moment the table grows, so it also drops the sessions that ran out
    /// unnoticed.
    pub(crate) async fn open_web_session(
        &self,
        token: TokenHash,
        user: UserId,
        policy: SessionPolicy,
    ) -> Result<Timestamp, StorageError> {
        self.call(move |connection, now| {
            let expires_at = later(now, policy.lifetime)?;
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM web_sessions WHERE expires_at <= ?1",
                [now.as_millisecond()],
            )?;
            transaction.execute(
                "INSERT INTO web_sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
                params![token.as_str(), user.as_str(), expires_at.as_millisecond()],
            )?;
            transaction.commit()?;
            Ok(expires_at)
        })
        .await
    }

    /// The live session a token hash names, renewed when less than the
    /// policy's margin remains; an expired session is deleted and is `None`.
    pub(crate) async fn resolve_web_session(
        &self,
        token: TokenHash,
        policy: SessionPolicy,
    ) -> Result<Option<ResolvedSession>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let found = {
                let mut statement = transaction.prepare_cached(
                    "SELECT s.expires_at, u.id, u.email, u.nickname, u.role, u.created_at
                     FROM web_sessions s JOIN users u ON u.id = s.user_id
                     WHERE s.token_hash = ?1",
                )?;
                let mut rows = statement.query([token.as_str()])?;
                match rows.next()? {
                    Some(row) => Some((instant(row, "web_sessions", "expires_at")?, user_row(row)?)),
                    None => None,
                }
            };
            let Some((expires_at, user)) = found else {
                return Ok(None);
            };
            if expires_at <= now {
                transaction.execute("DELETE FROM web_sessions WHERE token_hash = ?1", [token.as_str()])?;
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
            transaction.commit()?;
            Ok(Some(ResolvedSession {
                user,
                expires_at,
                renewed,
            }))
        })
        .await
    }

    pub(crate) async fn close_web_session(&self, token: TokenHash) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM web_sessions WHERE token_hash = ?1", [token.as_str()])?;
            Ok(())
        })
        .await
    }

    /// Stores a challenge in place of the user's previous one and answers
    /// when it expires; `None` while the previous one was sent less than the
    /// cooldown ago.
    pub(crate) async fn issue_email_challenge(
        &self,
        issue: ChallengeIssue,
        policy: ChallengePolicy,
    ) -> Result<Option<Timestamp>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let previous = {
                let mut statement =
                    transaction.prepare_cached("SELECT sent_at FROM email_challenges WHERE user_id = ?1")?;
                let mut rows = statement.query([issue.user.as_str()])?;
                match rows.next()? {
                    Some(row) => Some(instant(row, "email_challenges", "sent_at")?),
                    None => None,
                }
            };
            if previous.is_some_and(|sent_at| now.to_jiff().duration_since(sent_at.to_jiff()) < policy.cooldown) {
                return Ok(None);
            }
            let expires_at = later(now, policy.lifetime)?;
            transaction.execute(
                "INSERT INTO email_challenges
                   (user_id, id, email, password_hash, code_hash, expires_at, sent_at, attempts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
                 ON CONFLICT (user_id) DO UPDATE SET
                   id = excluded.id, email = excluded.email, password_hash = excluded.password_hash,
                   code_hash = excluded.code_hash, expires_at = excluded.expires_at,
                   sent_at = excluded.sent_at, attempts = 0",
                params![
                    issue.user.as_str(),
                    issue.id,
                    issue.email.as_str(),
                    issue.password_hash.as_str(),
                    issue.code_hash.as_str(),
                    expires_at.as_millisecond(),
                    now.as_millisecond()
                ],
            )?;
            transaction.commit()?;
            Ok(Some(expires_at))
        })
        .await
    }

    pub(crate) async fn delete_email_challenge(&self, user: UserId, id: String) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM email_challenges WHERE user_id = ?1 AND id = ?2",
                params![user.as_str(), id],
            )?;
            Ok(())
        })
        .await
    }

    /// Checks the code and, when it holds, changes the address and consumes
    /// the challenge, in one transaction with the uniqueness check.
    pub(crate) async fn confirm_email_challenge(
        &self,
        user: UserId,
        id: String,
        code_hash: CodeHash,
        attempts: u32,
    ) -> Result<ChallengeOutcome, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let Some(challenge) = pending_challenge(&transaction, &user, &id)? else {
                return Ok(ChallengeOutcome::InvalidCode);
            };
            // A password change since the challenge was issued ends it.
            let current: Option<String> = transaction
                .query_row("SELECT password_hash FROM users WHERE id = ?1", [user.as_str()], |row| row.get(0))
                .optional()?;
            if challenge.expires_at <= now
                || challenge.attempts >= attempts
                || current.as_deref() != Some(challenge.password_hash.as_str())
            {
                return Ok(ChallengeOutcome::InvalidCode);
            }
            if !code_hash.matches(&challenge.code_hash) {
                transaction.execute(
                    "UPDATE email_challenges SET attempts = attempts + 1 WHERE user_id = ?1",
                    [user.as_str()],
                )?;
                transaction.commit()?;
                return Ok(ChallengeOutcome::InvalidCode);
            }
            let taken: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM users WHERE email = ?1 AND id != ?2)",
                params![challenge.email.as_str(), user.as_str()],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(ChallengeOutcome::EmailTaken);
            }
            let changed = {
                let mut statement = transaction.prepare_cached(&format!(
                    "UPDATE users SET email = ?1 WHERE id = ?2 RETURNING {USER_COLUMNS}"
                ))?;
                let mut rows = statement.query(params![challenge.email.as_str(), user.as_str()])?;
                match rows.next()? {
                    Some(row) => user_row(row)?,
                    None => return Ok(ChallengeOutcome::InvalidCode),
                }
            };
            transaction.execute("DELETE FROM email_challenges WHERE user_id = ?1", [user.as_str()])?;
            transaction.commit()?;
            Ok(ChallengeOutcome::Changed(changed))
        })
        .await
    }
}

struct PendingChallenge {
    email: EmailAddress,
    password_hash: String,
    code_hash: String,
    expires_at: Timestamp,
    attempts: u32,
}

fn pending_challenge(
    transaction: &Transaction<'_>,
    user: &UserId,
    id: &str,
) -> Result<Option<PendingChallenge>, StorageError> {
    let mut statement = transaction.prepare_cached(
        "SELECT email, password_hash, code_hash, expires_at, attempts
         FROM email_challenges WHERE user_id = ?1 AND id = ?2",
    )?;
    let mut rows = statement.query(params![user.as_str(), id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(PendingChallenge {
        email: decode("email_challenges", "email", EmailAddress::try_from(row.get::<_, String>("email")?))?,
        password_hash: row.get("password_hash")?,
        code_hash: row.get("code_hash")?,
        expires_at: instant(row, "email_challenges", "expires_at")?,
        attempts: row.get("attempts")?,
    }))
}

fn saved_preferences(connection: &Connection, user: &UserId) -> Result<Preferences, StorageError> {
    let saved: Option<String> = connection
        .query_row(
            "SELECT preferences FROM user_preferences WHERE user_id = ?1",
            [user.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    match saved {
        Some(text) => json("user_preferences", "preferences", &text),
        None => Ok(Preferences::default()),
    }
}

/// A `users` row, read from its columns in `USER_COLUMNS`.
fn user_row(row: &Row<'_>) -> Result<UserDto, StorageError> {
    Ok(UserDto {
        id: decode("users", "id", UserId::try_from(row.get::<_, String>("id")?))?,
        email: decode("users", "email", EmailAddress::try_from(row.get::<_, String>("email")?))?,
        nickname: row.get("nickname")?,
        role: decode("users", "role", row.get::<_, String>("role")?.parse::<Role>())?,
        created_at: instant(row, "users", "created_at")?,
    })
}

fn account(row: &Row<'_>) -> Result<Account, StorageError> {
    Ok(Account {
        user: user_row(row)?,
        password_hash: decode("users", "password_hash", PasswordHash::parse(row.get("password_hash")?))?,
    })
}

/// `now` moved on by `by`.
pub(super) fn later(now: Timestamp, by: SignedDuration) -> Result<Timestamp, StorageError> {
    now.to_jiff()
        .checked_add(by)
        .map(Timestamp::truncate)
        .map_err(StorageError::Time)
}

/// What the tests of other modules start from.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// The instance's master account, created in `control`.
    pub(crate) async fn master(control: &ControlService) -> UserDto {
        let hash = PasswordHash::parse(
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo".to_owned(),
        )
        .unwrap();
        let email = EmailAddress::try_from("master@example.test".to_owned()).unwrap();
        control.create_master(email, hash).await.unwrap().unwrap()
    }

    /// Runs `sql` on the control database with text parameters, for a test
    /// that changes a row behind the service's back.
    pub(crate) async fn execute(control: &ControlService, sql: &'static str, parameters: Vec<String>) {
        control
            .call(move |connection, _| {
                connection.execute(sql, rusqlite::params_from_iter(parameters))?;
                Ok(())
            })
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::testing::master;
    use super::*;
    use crate::auth::sessions::{SESSION_POLICY, TokenHash};

    /// A clock the test sets.
    struct TestClock(Mutex<jiff::Timestamp>);

    impl Clock for TestClock {
        fn now(&self) -> Timestamp {
            Timestamp::truncate(*self.0.lock().unwrap())
        }
    }

    impl TestClock {
        fn advance(&self, by: SignedDuration) {
            let mut now = self.0.lock().unwrap();
            *now = now.checked_add(by).unwrap();
        }
    }

    #[tokio::test]
    async fn the_schema_applies_once_and_holds_no_conversation_data() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("control.sqlite");
        let first = ControlService::open(&path, Arc::new(demi_core::SystemClock)).await.unwrap();
        master(&first).await;
        first.close().await.unwrap();

        let again = ControlService::open(&path, Arc::new(demi_core::SystemClock)).await.unwrap();
        assert!(again.has_users().await.unwrap());
        let tables: Vec<String> = again
            .call(|connection, _| {
                let mut statement =
                    connection.prepare_cached("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
                let names = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
                Ok(names)
            })
            .await
            .unwrap();
        for table in ["users", "web_sessions", "devices", "workspaces", "conversations", "providers", "usage_ledger", "attachments"] {
            assert!(tables.iter().any(|name| name == table), "{table}");
        }
        for conversation_data in ["nodes", "blocks", "host_store"] {
            assert!(!tables.iter().any(|name| name == conversation_data), "{conversation_data}");
        }
        again.close().await.unwrap();
    }

    #[tokio::test]
    async fn expired_web_sessions_are_swept_when_a_session_opens() {
        let data = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock(Mutex::new(jiff::Timestamp::from_second(1_790_000_000).unwrap())));
        let control = ControlService::open(&data.path().join("control.sqlite"), clock.clone()).await.unwrap();
        let user = master(&control).await;
        for token in ["first", "second"] {
            control
                .open_web_session(TokenHash::of(token), user.id.clone(), SESSION_POLICY)
                .await
                .unwrap();
        }
        clock.advance(SESSION_POLICY.lifetime);
        control
            .open_web_session(TokenHash::of("third"), user.id.clone(), SESSION_POLICY)
            .await
            .unwrap();
        let sessions: i64 = control
            .call(|connection, _| {
                Ok(connection.query_row("SELECT COUNT(*) FROM web_sessions", [], |row| row.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(sessions, 1);
        let live = control
            .resolve_web_session(TokenHash::of("third"), SESSION_POLICY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(live.user, user);
        control.close().await.unwrap();
    }

    #[tokio::test]
    async fn preferences_are_each_users_own_and_a_corrupt_row_is_refused() {
        use demi_web_api::settings::{Appearance, PreferencesPatch, Theme};

        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
            .await
            .unwrap();
        let master = master(&control).await;
        let other = UserId::try_from("other-user").unwrap();
        let other_id = other.clone();
        control
            .call(move |connection, now| {
                connection.execute(
                    "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                     VALUES (?1, 'other@example.test', '', 'x', 'user', ?2)",
                    params![other_id.as_str(), now.as_millisecond()],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let dark = PreferencesPatch {
            appearance: Some(Appearance {
                theme: Some(Theme::Dark),
                ..Appearance::default()
            }),
            ..PreferencesPatch::default()
        };
        let saved = control
            .patch_preferences(master.id.clone(), settings::check(dark).unwrap())
            .await
            .unwrap();
        assert_eq!(saved.appearance.theme, Some(Theme::Dark));
        assert_eq!(control.preferences(master.id.clone()).await.unwrap(), saved);
        assert_eq!(control.preferences(other.clone()).await.unwrap(), Preferences::default());

        control
            .call(|connection, _| {
                connection.execute("UPDATE user_preferences SET preferences = '{\"appearance\":{}}'", [])?;
                Ok(())
            })
            .await
            .unwrap();
        let refused = control.preferences(master.id).await.unwrap_err();
        assert!(
            matches!(refused, StorageError::Corrupt { table: "user_preferences", column: "preferences", .. }),
            "{refused}"
        );
        control.close().await.unwrap();
    }
}
