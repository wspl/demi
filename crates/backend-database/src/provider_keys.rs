//! The keys another server borrows a provider entry with. The entry is a record
//! of its owner's core; the key is the server's, since it finds the owner.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::{ProviderId, UserId};
use rusqlite::{Connection, Row, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::{decode, instant};
use super::control::ControlService;

const SHARE_KEY_COLUMNS: &str = "id, owner_id, provider_id, created_by, created_at, used_by, used_at";

/// A share key of an entry, without its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareKeyRow {
    pub id: String,
    /// Whose core holds the entry.
    pub owner: UserId,
    pub provider: ProviderId,
    /// Who made it, whose management of the entry it lasts by.
    pub created_by: UserId,
    pub created_at: Timestamp,
    /// The server that used it last, by the host it gave, and when; none
    /// until it is used.
    pub used: Option<(String, Timestamp)>,
}

impl ControlService {
    /// Stores a new share key of `key.provider`. Its caller read that the
    /// entry exists.
    pub async fn insert_share_key(&self, token: TokenHash, key: ShareKeyRow) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO provider_share_keys (token_hash, id, owner_id, provider_id, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    token.as_str(),
                    key.id,
                    key.owner.as_str(),
                    key.provider.as_str(),
                    key.created_by.as_str(),
                    key.created_at.as_millisecond()
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// The entry's share keys, newest first, once the keys never used and
    /// made before `unused_since` are deleted.
    pub async fn share_keys(
        &self,
        owner: UserId,
        provider: ProviderId,
        unused_since: Timestamp,
    ) -> Result<Vec<ShareKeyRow>, StorageError> {
        self.call(move |connection, _| {
            prune_share_keys(connection, unused_since)?;
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {SHARE_KEY_COLUMNS} FROM provider_share_keys WHERE owner_id = ?1 AND provider_id = ?2
                 ORDER BY created_at DESC, id"
            ))?;
            let mut rows = statement.query([owner.as_str(), provider.as_str()])?;
            let mut keys = Vec::new();
            while let Some(row) = rows.next()? {
                keys.push(share_key_row(row)?);
            }
            Ok(keys)
        })
        .await
    }

    /// The share key whose SHA-256 is `token`, marked as used by `server` at
    /// `now`; none for an unknown key, and for one never used and made
    /// before `unused_since`, which is deleted.
    pub async fn use_share_key(
        &self,
        token: TokenHash,
        server: String,
        now: Timestamp,
        unused_since: Timestamp,
    ) -> Result<Option<ShareKeyRow>, StorageError> {
        self.call(move |connection, _| {
            prune_share_keys(connection, unused_since)?;
            let mut statement = connection.prepare_cached(&format!(
                "UPDATE provider_share_keys SET used_by = ?2, used_at = ?3 WHERE token_hash = ?1
                 RETURNING {SHARE_KEY_COLUMNS}"
            ))?;
            let mut rows = statement.query(params![token.as_str(), server, now.as_millisecond()])?;
            rows.next()?.map(share_key_row).transpose()
        })
        .await
    }

    /// Deletes the entry's share key `id`; false when it has none.
    pub async fn delete_share_key(&self, owner: UserId, provider: ProviderId, id: String) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let deleted = connection.execute(
                "DELETE FROM provider_share_keys WHERE owner_id = ?1 AND provider_id = ?2 AND id = ?3",
                params![owner.as_str(), provider.as_str(), id],
            )?;
            Ok(deleted == 1)
        })
        .await
    }

    /// Deletes every share key of the entry, as its deletion does.
    pub async fn delete_share_keys_of(&self, owner: UserId, provider: ProviderId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM provider_share_keys WHERE owner_id = ?1 AND provider_id = ?2",
                params![owner.as_str(), provider.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// The master account, which a shared instance's entries belong to.
    pub async fn master(&self) -> Result<Option<UserId>, StorageError> {
        self.call(|connection, _| {
            let id: Option<String> = rusqlite::OptionalExtension::optional(connection.query_row(
                "SELECT id FROM users WHERE role = 'master'",
                [],
                |row| row.get(0),
            ))?;
            id.map(|id| decode("users", "id", UserId::try_from(id))).transpose()
        })
        .await
    }
}

fn share_key_row(row: &Row<'_>) -> Result<ShareKeyRow, StorageError> {
    let used_by: Option<String> = row.get("used_by")?;
    let used = match used_by {
        Some(server) => Some((server, instant(row, "provider_share_keys", "used_at")?)),
        None => None,
    };
    Ok(ShareKeyRow {
        id: row.get("id")?,
        owner: decode(
            "provider_share_keys",
            "owner_id",
            UserId::try_from(row.get::<_, String>("owner_id")?),
        )?,
        provider: decode(
            "provider_share_keys",
            "provider_id",
            ProviderId::try_from(row.get::<_, String>("provider_id")?),
        )?,
        created_by: decode(
            "provider_share_keys",
            "created_by",
            UserId::try_from(row.get::<_, String>("created_by")?),
        )?,
        created_at: instant(row, "provider_share_keys", "created_at")?,
        used,
    })
}

/// Deletes the share keys never used and made before `unused_since`.
fn prune_share_keys(connection: &Connection, unused_since: Timestamp) -> Result<(), StorageError> {
    connection.execute(
        "DELETE FROM provider_share_keys WHERE used_at IS NULL AND created_at < ?1",
        [unused_since.as_millisecond()],
    )?;
    Ok(())
}
