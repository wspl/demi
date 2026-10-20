//! The users' passkeys and the challenges of their sign-ins and additions
//! (`crates/backend/src/config/secret.rs`,
//! `crates/backend-accounts/src/passkeys.rs`). A passkey is stored as
//! `webauthn-rs` encodes it, and a challenge as the state `webauthn-rs`
//! finishes it with; the accounts service decides when each is written.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::passkeys::{PasskeyChallengeId, PasskeyDto, PasskeyId};
use jiff::SignedDuration;
use rusqlite::{OptionalExtension, Row, params};
use webauthn_rs::prelude::{Credential, DiscoverableAuthentication, Passkey, PasskeyRegistration};

use super::StorageError;
use super::columns::{decode, instant, to_json};
use super::control::{ControlService, later};

const PASSKEYS: &str = "passkeys";
const CHALLENGES: &str = "passkey_challenges";
const COLUMNS: &str = "id, user_id, name, credential, created_at, last_used_at";

/// A passkey of an account.
#[derive(Debug, Clone)]
pub struct PasskeyRecord {
    pub id: PasskeyId,
    pub user: UserId,
    pub name: String,
    pub credential: Passkey,
    pub created_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
}

impl PasskeyRecord {
    /// The passkey as Settings lists it: synced when it last said it is
    /// backed up.
    pub fn dto(&self) -> PasskeyDto {
        let credential = Credential::from(self.credential.clone());
        PasskeyDto {
            id: self.id.clone(),
            name: self.name.clone(),
            synced: credential.backup_state,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
        }
    }
}

/// What a challenge finishes: a sign-in, which names no account until it is
/// signed, or an addition to the account of `user`.
#[derive(Debug, Clone)]
pub enum ChallengeState {
    SignIn(DiscoverableAuthentication),
    Registration {
        user: UserId,
        state: PasskeyRegistration,
    },
}

/// How long a challenge lasts, and how many may wait at once.
#[derive(Debug, Clone, Copy)]
pub struct ChallengeLimits {
    pub lifetime: SignedDuration,
    pub waiting: u32,
}

impl ControlService {
    /// Keeps a new challenge, after removing the expired ones; `false`,
    /// keeping nothing, when `limits.waiting` challenges wait already.
    pub async fn issue_passkey_challenge(
        &self,
        id: PasskeyChallengeId,
        challenge: ChallengeState,
        limits: ChallengeLimits,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let expires_at = later(now, limits.lifetime)?;
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM passkey_challenges WHERE expires_at <= ?1",
                [now.as_millisecond()],
            )?;
            let waiting: u32 =
                transaction.query_row("SELECT count(*) FROM passkey_challenges", [], |row| row.get(0))?;
            if waiting >= limits.waiting {
                transaction.commit()?;
                return Ok(false);
            }
            let (kind, user, state) = match &challenge {
                ChallengeState::SignIn(state) => ("sign_in", None, to_json(state)),
                ChallengeState::Registration { user, state } => {
                    ("registration", Some(user.as_str()), to_json(state))
                }
            };
            transaction.execute(
                "INSERT INTO passkey_challenges (id, kind, user_id, state, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id.as_str(), kind, user, state, expires_at.as_millisecond()],
            )?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Takes the challenge `id` once: it is removed, and answered unless it
    /// expired.
    pub async fn take_passkey_challenge(
        &self,
        id: PasskeyChallengeId,
    ) -> Result<Option<ChallengeState>, StorageError> {
        self.call(move |connection, now| {
            let taken: Option<(String, Option<String>, String, i64)> = connection
                .query_row(
                    "DELETE FROM passkey_challenges WHERE id = ?1
                     RETURNING kind, user_id, state, expires_at",
                    [id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((kind, user, state, expires_at)) = taken else {
                return Ok(None);
            };
            if expires_at <= now.as_millisecond() {
                return Ok(None);
            }
            let challenge = match (kind.as_str(), user) {
                ("sign_in", None) => {
                    ChallengeState::SignIn(decode(CHALLENGES, "state", serde_json::from_str(&state))?)
                }
                ("registration", Some(user)) => ChallengeState::Registration {
                    user: decode(CHALLENGES, "user_id", UserId::try_from(user))?,
                    state: decode(CHALLENGES, "state", serde_json::from_str(&state))?,
                },
                (kind, _) => {
                    return Err(StorageError::Corrupt {
                        table: CHALLENGES,
                        column: "kind",
                        reason: format!("{kind} does not fit its user"),
                    });
                }
            };
            Ok(Some(challenge))
        })
        .await
    }

    /// `user`'s passkeys, oldest first.
    pub async fn passkeys(&self, user: UserId) -> Result<Vec<PasskeyRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM passkeys WHERE user_id = ?1 ORDER BY created_at, rowid"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut passkeys = Vec::new();
            while let Some(row) = rows.next()? {
                passkeys.push(passkey_row(row)?);
            }
            Ok(passkeys)
        })
        .await
    }

    /// The passkey `id`, whoever's it is.
    pub async fn passkey(&self, id: PasskeyId) -> Result<Option<PasskeyRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement =
                connection.prepare_cached(&format!("SELECT {COLUMNS} FROM passkeys WHERE id = ?1"))?;
            let mut rows = statement.query([id.as_str()])?;
            rows.next()?.map(passkey_row).transpose()
        })
        .await
    }

    /// Adds the passkey `id`, of the credential `credential`, to `user`'s
    /// account; `None`, adding nothing, when an account has a passkey of
    /// that id already.
    pub async fn add_passkey(
        &self,
        user: UserId,
        id: PasskeyId,
        name: String,
        credential: Passkey,
    ) -> Result<Option<PasskeyRecord>, StorageError> {
        self.call(move |connection, now| {
            let added = connection.execute(
                "INSERT INTO passkeys (id, user_id, name, credential, created_at, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL) ON CONFLICT (id) DO NOTHING",
                params![id.as_str(), user.as_str(), name, to_json(&credential), now.as_millisecond()],
            )?;
            Ok((added > 0).then(|| PasskeyRecord {
                id,
                user,
                name,
                credential,
                created_at: now,
                last_used_at: None,
            }))
        })
        .await
    }

    /// Records that the passkey signed in now, with its credential as the
    /// sign-in updated it, if it did.
    pub async fn passkey_used(
        &self,
        id: PasskeyId,
        updated: Option<Passkey>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            match updated {
                Some(credential) => connection.execute(
                    "UPDATE passkeys SET last_used_at = ?1, credential = ?2 WHERE id = ?3",
                    params![now.as_millisecond(), to_json(&credential), id.as_str()],
                )?,
                None => connection.execute(
                    "UPDATE passkeys SET last_used_at = ?1 WHERE id = ?2",
                    params![now.as_millisecond(), id.as_str()],
                )?,
            };
            Ok(())
        })
        .await
    }

    /// Renames `user`'s passkey `id`; `false` when the user has none of
    /// that id.
    pub async fn rename_passkey(
        &self,
        user: UserId,
        id: PasskeyId,
        name: String,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let renamed = connection.execute(
                "UPDATE passkeys SET name = ?1 WHERE id = ?2 AND user_id = ?3",
                params![name, id.as_str(), user.as_str()],
            )?;
            Ok(renamed > 0)
        })
        .await
    }

    /// Removes `user`'s passkey `id`; `false` when the user has none of
    /// that id.
    pub async fn remove_passkey(&self, user: UserId, id: PasskeyId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM passkeys WHERE id = ?1 AND user_id = ?2",
                params![id.as_str(), user.as_str()],
            )?;
            Ok(removed > 0)
        })
        .await
    }
}

/// The id of the passkey whose credential id is `raw`: the credential id in
/// base64url without padding, as the browser names it. None for an empty
/// one, which no authenticator makes.
pub fn credential_id(raw: &[u8]) -> Option<PasskeyId> {
    PasskeyId::try_from(URL_SAFE_NO_PAD.encode(raw)).ok()
}

fn passkey_row(row: &Row<'_>) -> Result<PasskeyRecord, StorageError> {
    let credential: String = row.get("credential")?;
    Ok(PasskeyRecord {
        id: decode(PASSKEYS, "id", PasskeyId::try_from(row.get::<_, String>("id")?))?,
        user: decode(PASSKEYS, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        name: row.get("name")?,
        credential: decode(PASSKEYS, "credential", serde_json::from_str(&credential))?,
        created_at: instant(row, PASSKEYS, "created_at")?,
        last_used_at: row
            .get::<_, Option<i64>>("last_used_at")?
            .map(|millisecond| decode(PASSKEYS, "last_used_at", Timestamp::from_millisecond(millisecond)))
            .transpose()?,
    })
}
