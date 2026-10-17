//! The provider records of the control store (`storage.md` § Control
//! records): entries with their configuration still sealed, subscription
//! accounts with their sealed secrets, versions and quota snapshots, and each
//! entry's model catalog record. Sealing and opening belong to the vault;
//! these operations store and read the sealed bytes.

use demi_shared_types::{ProviderModelList, QuotaSnapshot, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};
use demi_web_api_protocol::providers::{CredentialKind, Rotation};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;

/// An entry's cached catalog as the control store keeps it: model metadata
/// only, under the key of the configuration and account it was read for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogRecord {
    /// The digest of the entry's configuration and active account.
    #[garde(length(min = 1))]
    pub key: String,
    /// When the source last answered.
    #[garde(skip)]
    pub checked_at: Timestamp,
    #[garde(dive)]
    pub catalog: ProviderModelList,
}

/// What the control migration that re-encodes the API-key entries' sealed
/// configurations (`storage.md` § Schemas and migrations) works with: the
/// vault, which alone holds the key they are sealed under and knows their
/// format.
pub trait ResealConfigs: Send + Sync {
    /// Entry `id` of `family`'s configuration `sealed`, opened, decoded and
    /// checked in the format the schema before names, and sealed again in
    /// the format of the schema after it; an error, which never quotes the
    /// configuration, says why it cannot be read.
    fn provider_config(&self, id: &ProviderId, family: &str, sealed: &[u8]) -> Result<Vec<u8>, String>;
}

/// A database that holds no sealed configuration a migration re-encodes,
/// such as a conversation's.
pub struct NoSealedConfigs;

impl ResealConfigs for NoSealedConfigs {
    fn provider_config(&self, id: &ProviderId, _: &str, _: &[u8]) -> Result<Vec<u8>, String> {
        Err(format!("no key opens the configuration of entry {id} here"))
    }
}

/// A `providers` row: an entry with its configuration still sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRow {
    pub id: ProviderId,
    pub owner: UserId,
    pub family: String,
    pub kind: CredentialKind,
    pub label: String,
    /// An API-key entry's sealed configuration; a subscription entry has
    /// none.
    pub config: Option<Vec<u8>>,
    /// A subscription entry's active account.
    pub active: Option<CredentialId>,
    pub created_at: Timestamp,
    /// Which account each agent uses; `manual` for an API-key entry.
    pub rotation: Rotation,
    /// Whether the entry makes images when its provider can.
    pub image_generation: bool,
    /// Whether the entry makes videos when its provider can.
    pub video_generation: bool,
}

/// The media switches a change of an entry sets, each kind's when given
/// (`media.md` § Which entries make which media).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaSwitches {
    pub image: Option<bool>,
    pub video: Option<bool>,
}

/// An entry as it is first stored, with manual rotation.
#[derive(Debug, Clone)]
pub struct NewProvider {
    pub id: ProviderId,
    pub owner: UserId,
    pub family: String,
    pub kind: CredentialKind,
    pub label: String,
    pub config: Option<Vec<u8>>,
    pub active: Option<CredentialId>,
}

/// A `provider_credentials` row: one account with its secret still sealed.
#[derive(Debug, Clone, PartialEq)]
pub struct CredentialRow {
    pub id: CredentialId,
    pub identity_key: Option<String>,
    pub label: String,
    pub detail: Option<String>,
    pub source: String,
    /// The sealed secret document; none for an account whose sign-in is
    /// kept elsewhere, as a Claude Code account's CLI keeps it.
    pub secret: Option<Vec<u8>>,
    /// Advanced by every write of the secret.
    pub version: u64,
    pub quota: Option<QuotaSnapshot>,
    /// When the account was last stored or refreshed.
    pub updated_at: Timestamp,
    /// The vendor refused the account's sign-in.
    pub signed_out: bool,
}

/// An account as a write stores it; the record's time is the write's.
#[derive(Debug, Clone)]
pub struct CredentialWrite {
    pub id: CredentialId,
    pub identity_key: Option<String>,
    pub label: String,
    pub detail: Option<String>,
    pub source: String,
    pub secret: Option<Vec<u8>>,
}

const PROVIDER_COLUMNS: &str = "id, owner_user_id, provider_type, credential_kind, label, config, active_credential_id, created_at, rotation, image_generation, video_generation";
/// The columns a new entry stores; its rotation is the default, manual, its
/// Image Generation is on, and it joins the end of its owner's list.
const NEW_PROVIDER_COLUMNS: &str = "id, owner_user_id, provider_type, credential_kind, label, config, active_credential_id, created_at, position";
const CREDENTIAL_COLUMNS: &str =
    "id, identity_key, label, detail, source, secret, version, quota, updated_at, signed_out";

impl ControlService {
    /// The master account, which a shared instance's entries belong to.
    pub async fn master(&self) -> Result<Option<UserId>, StorageError> {
        self.call(|connection, _| {
            let id: Option<String> = connection
                .query_row("SELECT id FROM users WHERE role = 'master'", [], |row| {
                    row.get(0)
                })
                .optional()?;
            id.map(|id| decode("users", "id", UserId::try_from(id)))
                .transpose()
        })
        .await
    }

    /// Stores a new entry with its first accounts in one transaction. `None`
    /// when the owner already holds the family's subscription entry: then
    /// nothing is stored.
    pub async fn insert_provider(
        &self,
        provider: NewProvider,
        accounts: Vec<CredentialWrite>,
    ) -> Result<Option<ProviderRow>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let inserted = transaction
                .query_row(
                    &format!(
                        "INSERT INTO providers ({NEW_PROVIDER_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                           (SELECT coalesce(max(position) + 1, 0) FROM providers WHERE owner_user_id = ?2))
                         ON CONFLICT (owner_user_id, provider_type) WHERE credential_kind = 'subscription' DO NOTHING
                         RETURNING {PROVIDER_COLUMNS}"
                    ),
                    params![
                        provider.id.as_str(),
                        provider.owner.as_str(),
                        provider.family,
                        provider.kind.to_string(),
                        provider.label,
                        provider.config,
                        provider.active.as_ref().map(CredentialId::as_str),
                        now.as_millisecond()
                    ],
                    |row| Ok(provider_row(row)),
                )
                .optional()?
                .transpose()?;
            let Some(inserted) = inserted else {
                return Ok(None);
            };
            for account in &accounts {
                write_credential(&transaction, &inserted.id, account, now)?;
            }
            transaction.commit()?;
            Ok(Some(inserted))
        })
        .await
    }

    pub async fn provider(&self, id: ProviderId) -> Result<Option<ProviderRow>, StorageError> {
        self.call(move |connection, _| provider_by_id(connection, &id))
            .await
    }

    /// The owner's entries in the order of Settings › Providers.
    pub async fn providers(&self, owner: UserId) -> Result<Vec<ProviderRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {PROVIDER_COLUMNS} FROM providers WHERE owner_user_id = ?1 ORDER BY position, rowid"
            ))?;
            let mut rows = statement.query([owner.as_str()])?;
            let mut providers = Vec::new();
            while let Some(row) = rows.next()? {
                providers.push(provider_row(row)?);
            }
            Ok(providers)
        })
        .await
    }

    /// Replaces the entry's label, family, sealed configuration, rotation or
    /// media switches, and answers the entry as it now is; `None` for an
    /// entry that no longer exists.
    pub async fn update_provider(
        &self,
        id: ProviderId,
        label: Option<String>,
        family: Option<String>,
        config: Option<Vec<u8>>,
        rotation: Option<Rotation>,
        switches: MediaSwitches,
    ) -> Result<Option<ProviderRow>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if let Some(label) = label {
                transaction.execute(
                    "UPDATE providers SET label = ?1 WHERE id = ?2",
                    params![label, id.as_str()],
                )?;
            }
            if let Some(family) = family {
                transaction.execute(
                    "UPDATE providers SET provider_type = ?1 WHERE id = ?2",
                    params![family, id.as_str()],
                )?;
            }
            if let Some(config) = config {
                transaction.execute(
                    "UPDATE providers SET config = ?1 WHERE id = ?2",
                    params![config, id.as_str()],
                )?;
            }
            if let Some(rotation) = rotation {
                transaction.execute(
                    "UPDATE providers SET rotation = ?1 WHERE id = ?2",
                    params![rotation.to_string(), id.as_str()],
                )?;
            }
            if let Some(on) = switches.image {
                transaction.execute(
                    "UPDATE providers SET image_generation = ?1 WHERE id = ?2",
                    params![on, id.as_str()],
                )?;
            }
            if let Some(on) = switches.video {
                transaction.execute(
                    "UPDATE providers SET video_generation = ?1 WHERE id = ?2",
                    params![on, id.as_str()],
                )?;
            }
            let updated = provider_by_id(&transaction, &id)?;
            transaction.commit()?;
            Ok(updated)
        })
        .await
    }

    /// Puts the owner's entries in the order `ids` names them; `false`,
    /// changing nothing, unless `ids` names each of them once.
    pub async fn set_provider_order(&self, owner: UserId, ids: Vec<ProviderId>) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let held: Vec<String> = transaction
                .prepare("SELECT id FROM providers WHERE owner_user_id = ?1")?
                .query_map([owner.as_str()], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            let mut named: Vec<&str> = ids.iter().map(ProviderId::as_str).collect();
            named.sort_unstable();
            let mut sorted: Vec<&str> = held.iter().map(String::as_str).collect();
            sorted.sort_unstable();
            if named != sorted {
                return Ok(false);
            }
            for (position, id) in ids.iter().enumerate() {
                transaction.execute(
                    "UPDATE providers SET position = ?2 WHERE id = ?1",
                    params![id.as_str(), i64::try_from(position).expect("an owner holds few entries")],
                )?;
            }
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Deletes the entry with its accounts and catalog record.
    pub async fn delete_provider(&self, id: ProviderId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM providers WHERE id = ?1", [id.as_str()])?;
            Ok(())
        })
        .await
    }

    /// The entry's accounts in the list's order.
    pub async fn credentials(
        &self,
        provider: ProviderId,
    ) -> Result<Vec<CredentialRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {CREDENTIAL_COLUMNS} FROM provider_credentials WHERE provider_id = ?1 ORDER BY position, id"
            ))?;
            let mut rows = statement.query([provider.as_str()])?;
            let mut credentials = Vec::new();
            while let Some(row) = rows.next()? {
                credentials.push(credential_row(row)?);
            }
            Ok(credentials)
        })
        .await
    }

    pub async fn credential(
        &self,
        provider: ProviderId,
        id: CredentialId,
    ) -> Result<Option<CredentialRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {CREDENTIAL_COLUMNS} FROM provider_credentials WHERE provider_id = ?1 AND id = ?2"
            ))?;
            let mut rows = statement.query(params![provider.as_str(), id.as_str()])?;
            rows.next()?.map(credential_row).transpose()
        })
        .await
    }

    /// Inserts the account, or replaces the one with its id and advances its
    /// version, and selects it when the entry has no active account, all in
    /// one transaction; none for an entry that no longer exists.
    pub async fn write_credential(
        &self,
        provider: ProviderId,
        account: CredentialWrite,
    ) -> Result<Option<CredentialWritten>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let exists: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM providers WHERE id = ?1)",
                [provider.as_str()],
                |row| row.get(0),
            )?;
            if !exists {
                return Ok(None);
            }
            let written = write_credential(&transaction, &provider, &account, now)?;
            transaction.commit()?;
            Ok(Some(written))
        })
        .await
    }

    /// Stores a refreshed secret, or clears it, only while the account is
    /// still at `version`; `false` when another writer stored first.
    pub async fn replace_credential_secret(
        &self,
        provider: ProviderId,
        id: CredentialId,
        secret: Option<Vec<u8>>,
        version: u64,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let Ok(version) = i64::try_from(version) else {
                // Versions count writes from 1, so one beyond i64 was never
                // stored.
                return Ok(false);
            };
            let changed = connection.execute(
                "UPDATE provider_credentials SET secret = ?1, version = version + 1, updated_at = ?2
                 WHERE provider_id = ?3 AND id = ?4 AND version = ?5",
                params![secret, now.as_millisecond(), provider.as_str(), id.as_str(), version],
            )?;
            Ok(changed == 1)
        })
        .await
    }

    /// Marks the account as signed out by its vendor; an account removed
    /// meanwhile marks nothing. Answers whether it changed.
    pub async fn sign_out_credential(
        &self,
        provider: ProviderId,
        id: CredentialId,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE provider_credentials SET signed_out = 1
                 WHERE provider_id = ?1 AND id = ?2 AND signed_out = 0",
                params![provider.as_str(), id.as_str()],
            )?;
            Ok(changed == 1)
        })
        .await
    }

    /// Replaces the account's kept quota snapshot; an account removed
    /// meanwhile keeps nothing.
    pub async fn set_credential_quota(
        &self,
        provider: ProviderId,
        id: CredentialId,
        quota: std::sync::Arc<QuotaSnapshot>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE provider_credentials SET quota = ?1 WHERE provider_id = ?2 AND id = ?3",
                params![to_json(&*quota), provider.as_str(), id.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Removes the account. When it was the entry's active account, the
    /// entry's first remaining account in the list's order becomes active in
    /// the same transaction, or none when it was the last
    /// (`providers.md` § Login and publication). An entry left with fewer
    /// than two accounts has nothing to rotate, and its rotation goes back
    /// to Manual in the same transaction (`providers.md` § Account
    /// rotation). Answers the account active afterwards.
    pub async fn remove_credential(
        &self,
        provider: ProviderId,
        id: CredentialId,
    ) -> Result<Option<CredentialId>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM provider_credentials WHERE provider_id = ?1 AND id = ?2",
                params![provider.as_str(), id.as_str()],
            )?;
            transaction.execute(
                "UPDATE providers SET active_credential_id =
                   (SELECT id FROM provider_credentials WHERE provider_id = ?1 ORDER BY position, id LIMIT 1)
                 WHERE id = ?1 AND active_credential_id = ?2",
                params![provider.as_str(), id.as_str()],
            )?;
            transaction.execute(
                "UPDATE providers SET rotation = 'manual'
                 WHERE id = ?1
                   AND (SELECT count(*) FROM provider_credentials WHERE provider_id = ?1) < 2",
                [provider.as_str()],
            )?;
            let active: Option<String> = transaction.query_row(
                "SELECT active_credential_id FROM providers WHERE id = ?1",
                [provider.as_str()],
                |row| row.get(0),
            )?;
            transaction.commit()?;
            active
                .map(|id| {
                    decode(
                        "providers",
                        "active_credential_id",
                        CredentialId::try_from(id),
                    )
                })
                .transpose()
        })
        .await
    }

    /// Puts the entry's accounts in the order `ids` names them; `false`,
    /// and nothing changed, unless `ids` names every account of the entry
    /// once.
    pub async fn set_credential_order(
        &self,
        provider: ProviderId,
        ids: Vec<CredentialId>,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let mut stored: Vec<String> = transaction
                .prepare_cached("SELECT id FROM provider_credentials WHERE provider_id = ?1")?
                .query_map([provider.as_str()], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let mut named: Vec<&str> = ids.iter().map(CredentialId::as_str).collect();
            stored.sort();
            named.sort_unstable();
            if stored != named {
                return Ok(false);
            }
            for (position, id) in ids.iter().enumerate() {
                transaction.execute(
                    "UPDATE provider_credentials SET position = ?3 WHERE provider_id = ?1 AND id = ?2",
                    params![provider.as_str(), id.as_str(), i64::try_from(position).expect("an entry holds few accounts")],
                )?;
            }
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Selects the account the entry infers with; `false` when the entry
    /// holds no such account.
    pub async fn set_active_credential(
        &self,
        provider: ProviderId,
        id: CredentialId,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE providers SET active_credential_id = ?2 WHERE id = ?1 AND EXISTS
                   (SELECT 1 FROM provider_credentials WHERE provider_id = ?1 AND id = ?2)",
                params![provider.as_str(), id.as_str()],
            )?;
            Ok(changed == 1)
        })
        .await
    }

    /// The entry's catalog record, validated.
    pub async fn catalog_record(
        &self,
        provider: ProviderId,
    ) -> Result<Option<CatalogRecord>, StorageError> {
        self.call(move |connection, _| {
            let text: Option<String> = connection
                .query_row(
                    "SELECT record FROM model_catalogs WHERE provider_id = ?1",
                    [provider.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            text.map(|text| json("model_catalogs", "record", &text))
                .transpose()
        })
        .await
    }

    /// Stores the entry's catalog record in place of the last one; an entry
    /// deleted meanwhile keeps nothing.
    pub async fn put_catalog_record(
        &self,
        provider: ProviderId,
        record: CatalogRecord,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO model_catalogs (provider_id, record)
                 SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM providers WHERE id = ?1)
                 ON CONFLICT (provider_id) DO UPDATE SET record = excluded.record",
                params![provider.as_str(), to_json(&record)],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn delete_catalog_record(&self, provider: ProviderId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM model_catalogs WHERE provider_id = ?1",
                [provider.as_str()],
            )?;
            Ok(())
        })
        .await
    }
}

fn provider_by_id(
    connection: &Connection,
    id: &ProviderId,
) -> Result<Option<ProviderRow>, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {PROVIDER_COLUMNS} FROM providers WHERE id = ?1"
    ))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(provider_row).transpose()
}

/// Inserts or replaces an account, advancing its version, and selects it
/// when the entry has no active account. A new account goes to the end of
/// the list; a replaced one keeps its place.
fn write_credential(
    connection: &Connection,
    provider: &ProviderId,
    account: &CredentialWrite,
    now: Timestamp,
) -> Result<CredentialWritten, StorageError> {
    connection.execute(
        "INSERT INTO provider_credentials
           (provider_id, id, identity_key, label, detail, source, secret, version, updated_at, position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8,
           (SELECT coalesce(max(position) + 1, 0) FROM provider_credentials WHERE provider_id = ?1))
         ON CONFLICT (provider_id, id) DO UPDATE SET
           identity_key = excluded.identity_key, label = excluded.label, detail = excluded.detail,
           source = excluded.source, secret = excluded.secret, version = version + 1,
           updated_at = excluded.updated_at, signed_out = 0",
        params![
            provider.as_str(),
            account.id.as_str(),
            account.identity_key,
            account.label,
            account.detail,
            account.source,
            account.secret,
            now.as_millisecond()
        ],
    )?;
    let selected = connection.execute(
        "UPDATE providers SET active_credential_id = ?2 WHERE id = ?1 AND active_credential_id IS NULL",
        params![provider.as_str(), account.id.as_str()],
    )?;
    Ok(CredentialWritten {
        activated: selected > 0,
    })
}

/// How a stored account changed its entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialWritten {
    /// The entry had no active account, and this one became it.
    pub activated: bool,
}

fn provider_row(row: &Row<'_>) -> Result<ProviderRow, StorageError> {
    let active: Option<String> = row.get("active_credential_id")?;
    Ok(ProviderRow {
        id: decode(
            "providers",
            "id",
            ProviderId::try_from(row.get::<_, String>("id")?),
        )?,
        owner: decode(
            "providers",
            "owner_user_id",
            UserId::try_from(row.get::<_, String>("owner_user_id")?),
        )?,
        family: row.get("provider_type")?,
        kind: decode(
            "providers",
            "credential_kind",
            row.get::<_, String>("credential_kind")?
                .parse::<CredentialKind>(),
        )?,
        label: row.get("label")?,
        config: row.get("config")?,
        active: active
            .map(|id| {
                decode(
                    "providers",
                    "active_credential_id",
                    CredentialId::try_from(id),
                )
            })
            .transpose()?,
        created_at: instant(row, "providers", "created_at")?,
        rotation: decode(
            "providers",
            "rotation",
            row.get::<_, String>("rotation")?.parse::<Rotation>(),
        )?,
        image_generation: row.get("image_generation")?,
        video_generation: row.get("video_generation")?,
    })
}

fn credential_row(row: &Row<'_>) -> Result<CredentialRow, StorageError> {
    let quota: Option<String> = row.get("quota")?;
    let version: i64 = row.get("version")?;
    Ok(CredentialRow {
        id: decode(
            "provider_credentials",
            "id",
            CredentialId::try_from(row.get::<_, String>("id")?),
        )?,
        identity_key: row.get("identity_key")?,
        label: row.get("label")?,
        detail: row.get("detail")?,
        source: row.get("source")?,
        secret: row.get("secret")?,
        version: decode("provider_credentials", "version", u64::try_from(version))?,
        quota: quota
            .map(|text| json("provider_credentials", "quota", &text))
            .transpose()?,
        updated_at: instant(row, "provider_credentials", "updated_at")?,
        signed_out: row.get("signed_out")?,
    })
}
