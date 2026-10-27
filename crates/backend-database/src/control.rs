//! The control service: every read and write of `control.sqlite`, each one
//! closure and one transaction on the database's own thread.
//!
//! Operations take owned, serializable input, so no caller holds a transaction
//! open across a wait and a control service in another process could serve them
//! unchanged.

use std::path::Path;
use std::sync::Arc;

use demi_shared_types::{Clock, Timestamp};
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::server_upgrade::UpgradeChannel;
use demi_web_api_protocol::settings::Preferences;
use demi_web_api_protocol::text::EmailAddress;
use demi_web_api_protocol::users::{CloudStorage, ListedUser};
use jiff::SignedDuration;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use tokio::sync::broadcast;

use super::accounts::{
    Account, ChallengeIssue, ChallengeOutcome, ChallengePolicy, CodeHash, PasswordHash, TokenHash,
};
use super::columns::{decode, instant, json, to_json};
use super::providers::ResealSealed;
use super::usage::UsageWritten;
use super::{StorageError, schema, sqlite};

#[derive(Clone)]
pub struct ControlService {
    db: tokio_rusqlite::Connection,
    clock: Arc<dyn Clock>,
    /// Each usage ledger row as it is written, for whoever shows its
    /// conversation's cost (`crates/backend-database/src/usage.rs`).
    usage_written: broadcast::Sender<UsageWritten>,
}

/// How many written rows wait for a follower that fell behind before it
/// misses the oldest.
///
/// Bounded because the rows wait in memory for each follower, and a follower
/// that never reads would grow them without end.
const USAGE_WAITING: usize = 1024;

/// What `user_row` reads.
pub(crate) const USER_COLUMNS: &str = "users.id, users.email, users.nickname, users.role, users.created_at,
     users.disabled, users.chooses_password";

impl ControlService {
    /// Opens the database: gives a new one its schema, and migrates one of
    /// an earlier schema, re-encoding its sealed values with `seals`.
    pub async fn open(
        path: &Path,
        clock: Arc<dyn Clock>,
        seals: Arc<dyn ResealSealed>,
    ) -> Result<Self, StorageError> {
        let db = tokio_rusqlite::Connection::open(path).await?;
        let control = Self {
            db,
            clock,
            usage_written: broadcast::channel(USAGE_WAITING).0,
        };
        let path = path.to_owned();
        control
            .call(move |connection, _| {
                sqlite::configure(connection)?;
                schema::CONTROL.apply_sealed(connection, &path, seals.as_ref())
            })
            .await?;
        Ok(control)
    }

    /// Each usage ledger row written from now on.
    pub fn usage_written(&self) -> broadcast::Receiver<UsageWritten> {
        self.usage_written.subscribe()
    }

    pub(crate) fn announce_usage(&self, written: UsageWritten) {
        // No follower is no one to tell: the backend follows from its start.
        let _ = self.usage_written.send(written);
    }

    /// The wall-clock time the records it writes carry.
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Closes the database; operations after it fail with `Closed`.
    pub async fn close(&self) -> Result<(), StorageError> {
        sqlite::close(self.db.clone()).await
    }

    /// Runs `work` on the database thread with the time of the operation, in
    /// the whole milliseconds storage keeps.
    pub(crate) async fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection, Timestamp) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, StorageError> {
        let clock = self.clock.clone();
        self.db
            .call(move |connection| work(connection, clock.now()))
            .await
            .map_err(sqlite::flatten)
    }

    pub async fn has_users(&self) -> Result<bool, StorageError> {
        self.call(|connection, _| {
            let found = connection
                .query_row("SELECT EXISTS (SELECT 1 FROM users)", [], |row| row.get(0))?;
            Ok(found)
        })
        .await
    }

    /// The instance's first account, created only while there is no account
    /// at all; `None` once setup has run.
    pub async fn create_master(
        &self,
        nickname: String,
        email: EmailAddress,
        password_hash: PasswordHash,
    ) -> Result<Option<UserDto>, StorageError> {
        let id = UserId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let set_up: bool =
                transaction
                    .query_row("SELECT EXISTS (SELECT 1 FROM users)", [], |row| row.get(0))?;
            if set_up {
                return Ok(None);
            }
            transaction.execute(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id.as_str(),
                    email.as_str(),
                    nickname,
                    password_hash.as_str(),
                    Role::Master.to_string(),
                    now.as_millisecond()
                ],
            )?;
            transaction.commit()?;
            Ok(Some(UserDto {
                id,
                email,
                nickname,
                role: Role::Master,
                created_at: now,
                disabled: false,
                chooses_password: false,
            }))
        })
        .await
    }

    /// The login lookup.
    pub async fn account_by_email(
        &self,
        email: EmailAddress,
    ) -> Result<Option<Account>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {USER_COLUMNS}, password_hash FROM users WHERE email = ?1"
            ))?;
            let mut rows = statement.query([email.as_str()])?;
            rows.next()?.map(account).transpose()
        })
        .await
    }

    pub async fn account(&self, user: UserId) -> Result<Option<Account>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {USER_COLUMNS}, password_hash FROM users WHERE id = ?1"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            rows.next()?.map(account).transpose()
        })
        .await
    }

    /// Every account, in the order they were created, each with when it was
    /// last active and the size of its Cloud disks.
    pub async fn users(&self) -> Result<Vec<ListedUser>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {USER_COLUMNS}, users.last_active_at, users.cloud_system_gb, users.cloud_home_gb
                 FROM users ORDER BY users.created_at, users.rowid"
            ))?;
            let mut rows = statement.query([])?;
            let mut users = Vec::new();
            while let Some(row) = rows.next()? {
                users.push(ListedUser {
                    user: user_row(row)?,
                    last_active_at: row
                        .get::<_, Option<i64>>("last_active_at")?
                        .map(|millisecond| decode("web_sessions", "last_active_at", Timestamp::from_millisecond(millisecond)))
                        .transpose()?,
                    cloud_storage: cloud_storage_row(row)?,
                });
            }
            Ok(users)
        })
        .await
    }

    /// A new account of `role` whose owner is still to choose its own
    /// password; `None`, writing nothing, when an account has the address
    /// already.
    pub async fn create_user(
        &self,
        email: EmailAddress,
        password_hash: PasswordHash,
        role: Role,
    ) -> Result<Option<UserDto>, StorageError> {
        let id = UserId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            let created = connection.execute(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at, disabled, chooses_password)
                 VALUES (?1, ?2, '', ?3, ?4, ?5, 0, 1) ON CONFLICT (email) DO NOTHING",
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
                disabled: false,
                chooses_password: true,
            }))
        })
        .await
    }

    pub async fn email_in_use(&self, email: EmailAddress) -> Result<bool, StorageError> {
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
    pub async fn set_nickname(
        &self,
        user: UserId,
        nickname: String,
    ) -> Result<Option<UserDto>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "UPDATE users SET nickname = ?1 WHERE id = ?2 RETURNING {USER_COLUMNS}"
            ))?;
            let mut rows = statement.query(params![nickname, user.as_str()])?;
            rows.next()?.map(user_row).transpose()
        })
        .await
    }

    /// Sets the password its owner chose, which ends the account's choice.
    pub async fn set_chosen_password(
        &self,
        user: UserId,
        password_hash: PasswordHash,
    ) -> Result<(), StorageError> {
        self.set_password(user, password_hash, false).await
    }

    /// Sets a first password an administrator's reset made, which its owner
    /// replaces at once: the account chooses its own password again.
    pub async fn set_first_password(
        &self,
        user: UserId,
        password_hash: PasswordHash,
    ) -> Result<(), StorageError> {
        self.set_password(user, password_hash, true).await
    }

    async fn set_password(
        &self,
        user: UserId,
        password_hash: PasswordHash,
        chooses: bool,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE users SET password_hash = ?1, chooses_password = ?2 WHERE id = ?3",
                params![password_hash.as_str(), chooses, user.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The size of the account's Cloud disks; none for an id that names no
    /// account.
    pub async fn cloud_storage(&self, user: UserId) -> Result<Option<CloudStorage>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT cloud_system_gb, cloud_home_gb FROM users WHERE id = ?1",
            )?;
            let mut rows = statement.query([user.as_str()])?;
            rows.next()?.map(cloud_storage_row).transpose()
        })
        .await
    }

    /// Sets the size of the account's Cloud disks, which its Cloud's next
    /// start takes.
    pub async fn set_cloud_storage(&self, user: UserId, storage: CloudStorage) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE users SET cloud_system_gb = ?1, cloud_home_gb = ?2 WHERE id = ?3",
                params![storage.system_gb, storage.home_gb, user.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Gives the account `role`.
    pub async fn set_role(&self, user: UserId, role: Role) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE users SET role = ?1 WHERE id = ?2",
                params![role.to_string(), user.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Disables or enables the account. Disabling deletes its sessions in
    /// the same transaction and answers their tokens' hashes, whose pages'
    /// channels and sockets end; enabling answers none.
    pub async fn set_disabled(
        &self,
        user: UserId,
        disabled: bool,
    ) -> Result<Vec<TokenHash>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "UPDATE users SET disabled = ?1 WHERE id = ?2",
                params![disabled, user.as_str()],
            )?;
            let ended = if disabled {
                sign_out_all(&transaction, &user)?
            } else {
                Vec::new()
            };
            transaction.commit()?;
            Ok(ended)
        })
        .await
    }

    /// Deletes every session of the account and answers their tokens'
    /// hashes, whose pages' channels and sockets end.
    pub async fn sign_out_everywhere(&self, user: UserId) -> Result<Vec<TokenHash>, StorageError> {
        self.call(move |connection, _| sign_out_all(connection, &user))
            .await
    }

    /// The tokens' hashes of every live session of the account, whose
    /// channels and sockets end without the sessions: the account must
    /// choose its password first.
    pub async fn session_tokens(&self, user: UserId) -> Result<Vec<TokenHash>, StorageError> {
        self.call(move |connection, now| {
            let mut statement = connection.prepare_cached(
                "SELECT token_hash FROM web_sessions WHERE user_id = ?1 AND expires_at > ?2",
            )?;
            let tokens = statement
                .query_map(params![user.as_str(), now.as_millisecond()], |row| {
                    row.get::<_, String>(0)
                })?
                .map(|token| token.map(TokenHash::from_stored))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(tokens)
        })
        .await
    }

    /// The channel the server's updates come from: `installed`, the
    /// installation's, until an administrator chose another.
    pub async fn upgrade_channel(&self, installed: UpgradeChannel) -> Result<UpgradeChannel, StorageError> {
        self.call(move |connection, _| Ok(chosen_upgrade_channel(connection)?.unwrap_or(installed)))
            .await
    }

    pub async fn set_upgrade_channel(&self, channel: UpgradeChannel) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO server_settings (id, upgrade_channel) VALUES (1, ?1)
                 ON CONFLICT (id) DO UPDATE SET upgrade_channel = excluded.upgrade_channel",
                [channel.to_string()],
            )?;
            Ok(())
        })
        .await
    }

    /// The user's saved preferences; a user who saved none has no overrides.
    pub async fn preferences(&self, user: UserId) -> Result<Preferences, StorageError> {
        self.call(move |connection, _| saved_preferences(connection, &user))
            .await
    }

    /// Merges a patch into the user's preferences with `merge`, which takes
    /// the saved preferences and answers them patched, and answers them as
    /// saved. The read, the merge and the write are one transaction, so
    /// patches of different fields that arrive together all stay.
    pub async fn patch_preferences(
        &self,
        user: UserId,
        merge: impl FnOnce(Preferences) -> Preferences + Send + 'static,
    ) -> Result<Preferences, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let preferences = merge(saved_preferences(&transaction, &user)?);
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

    /// Stores a challenge in place of the user's previous one and answers
    /// when it expires; `None` while the previous one was sent less than the
    /// cooldown ago.
    pub async fn issue_email_challenge(
        &self,
        issue: ChallengeIssue,
        policy: ChallengePolicy,
    ) -> Result<Option<Timestamp>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let previous = {
                let mut statement = transaction
                    .prepare_cached("SELECT sent_at FROM email_challenges WHERE user_id = ?1")?;
                let mut rows = statement.query([issue.user.as_str()])?;
                match rows.next()? {
                    Some(row) => Some(instant(row, "email_challenges", "sent_at")?),
                    None => None,
                }
            };
            if previous.is_some_and(|sent_at| {
                now.to_jiff().duration_since(sent_at.to_jiff()) < policy.cooldown
            }) {
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

    pub async fn delete_email_challenge(
        &self,
        user: UserId,
        id: String,
    ) -> Result<(), StorageError> {
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
    pub async fn confirm_email_challenge(
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
                .query_row(
                    "SELECT password_hash FROM users WHERE id = ?1",
                    [user.as_str()],
                    |row| row.get(0),
                )
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
            transaction.execute(
                "DELETE FROM email_challenges WHERE user_id = ?1",
                [user.as_str()],
            )?;
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
        email: decode(
            "email_challenges",
            "email",
            EmailAddress::try_from(row.get::<_, String>("email")?),
        )?,
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
pub(crate) fn user_row(row: &Row<'_>) -> Result<UserDto, StorageError> {
    Ok(UserDto {
        id: decode("users", "id", UserId::try_from(row.get::<_, String>("id")?))?,
        email: decode(
            "users",
            "email",
            EmailAddress::try_from(row.get::<_, String>("email")?),
        )?,
        nickname: row.get("nickname")?,
        role: decode(
            "users",
            "role",
            row.get::<_, String>("role")?.parse::<Role>(),
        )?,
        created_at: instant(row, "users", "created_at")?,
        disabled: row.get("disabled")?,
        chooses_password: row.get("chooses_password")?,
    })
}

/// The size of an account's Cloud disks, from its row's `cloud_system_gb`
/// and `cloud_home_gb`.
fn cloud_storage_row(row: &Row<'_>) -> Result<CloudStorage, StorageError> {
    let storage = CloudStorage {
        system_gb: row.get("cloud_system_gb")?,
        home_gb: row.get("cloud_home_gb")?,
    };
    decode("users", "cloud_system_gb", garde::Validate::validate(&storage).map(|()| storage))
}

/// Deletes the user's sessions and answers their tokens' hashes.
fn sign_out_all(connection: &Connection, user: &UserId) -> Result<Vec<TokenHash>, StorageError> {
    let mut statement = connection
        .prepare_cached("DELETE FROM web_sessions WHERE user_id = ?1 RETURNING token_hash")?;
    let tokens = statement
        .query_map([user.as_str()], |row| row.get::<_, String>(0))?
        .map(|token| token.map(TokenHash::from_stored))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tokens)
}

fn account(row: &Row<'_>) -> Result<Account, StorageError> {
    Ok(Account {
        user: user_row(row)?,
        password_hash: decode(
            "users",
            "password_hash",
            PasswordHash::parse(row.get("password_hash")?),
        )?,
    })
}

/// `now` moved on by `by`.
pub(crate) fn later(now: Timestamp, by: SignedDuration) -> Result<Timestamp, StorageError> {
    now.to_jiff()
        .checked_add(by)
        .map(Timestamp::truncate)
        .map_err(StorageError::Time)
}

/// The channel an administrator chose in Settings › Updates, if one did.
fn chosen_upgrade_channel(connection: &Connection) -> Result<Option<UpgradeChannel>, StorageError> {
    let channel: Option<String> = connection
        .query_row("SELECT upgrade_channel FROM server_settings WHERE id = 1", [], |row| row.get(0))
        .optional()?;
    channel
        .map(|channel| {
            channel.parse().map_err(|_| StorageError::Corrupt {
                table: "server_settings",
                column: "upgrade_channel",
                reason: format!("{channel} is no channel"),
            })
        })
        .transpose()
}

/// The channel an administrator chose in Settings › Updates, read from the
/// control database at `path` without changing it, while the backend may
/// run; none when no one chose one, or when the database or its table does
/// not exist yet.
pub fn stored_upgrade_channel(path: &Path) -> Result<Option<UpgradeChannel>, StorageError> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let settings: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'server_settings')",
        [],
        |row| row.get(0),
    )?;
    if !settings {
        return Ok(None);
    }
    chosen_upgrade_channel(&connection)
}
