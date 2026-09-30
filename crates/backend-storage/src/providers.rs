//! The provider records of the control store (`storage.md` § Control
//! records): entries with their configuration still sealed, subscription
//! accounts with their sealed secrets, versions and quota snapshots, and each
//! entry's model catalog record. Sealing and opening belong to the vault;
//! these operations store and read the sealed bytes.

use demi_core::{ProviderModelList, QuotaSnapshot, Timestamp};
use demi_web_api::ids::{CredentialId, ProviderId, UserId};
use demi_web_api::providers::CredentialKind;
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
}

/// An entry as it is first stored.
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
    pub secret: Vec<u8>,
    /// Advanced by every write of the secret.
    pub version: u64,
    pub quota: Option<QuotaSnapshot>,
    /// When the account was last stored or refreshed.
    pub updated_at: Timestamp,
}

/// An account as a write stores it; the record's time is the write's.
#[derive(Debug, Clone)]
pub struct CredentialWrite {
    pub id: CredentialId,
    pub identity_key: Option<String>,
    pub label: String,
    pub detail: Option<String>,
    pub source: String,
    pub secret: Vec<u8>,
}

const PROVIDER_COLUMNS: &str =
    "id, owner_user_id, provider_type, credential_kind, label, config, active_credential_id, created_at";
const CREDENTIAL_COLUMNS: &str = "id, identity_key, label, detail, source, secret, version, quota, updated_at";

impl ControlService {
    /// The master account, which a shared instance's entries belong to.
    pub async fn master(&self) -> Result<Option<UserId>, StorageError> {
        self.call(|connection, _| {
            let id: Option<String> = connection
                .query_row("SELECT id FROM users WHERE role = 'master'", [], |row| row.get(0))
                .optional()?;
            id.map(|id| decode("users", "id", UserId::try_from(id))).transpose()
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
                        "INSERT INTO providers ({PROVIDER_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
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
        self.call(move |connection, _| provider_by_id(connection, &id)).await
    }

    /// The owner's entries, oldest first.
    pub async fn providers(&self, owner: UserId) -> Result<Vec<ProviderRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {PROVIDER_COLUMNS} FROM providers WHERE owner_user_id = ?1 ORDER BY created_at, rowid"
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

    /// Replaces the entry's label or sealed configuration, and answers the
    /// entry as it now is; `None` for an entry that no longer exists.
    pub async fn update_provider(
        &self,
        id: ProviderId,
        label: Option<String>,
        config: Option<Vec<u8>>,
    ) -> Result<Option<ProviderRow>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if let Some(label) = label {
                transaction.execute(
                    "UPDATE providers SET label = ?1 WHERE id = ?2",
                    params![label, id.as_str()],
                )?;
            }
            if let Some(config) = config {
                transaction.execute(
                    "UPDATE providers SET config = ?1 WHERE id = ?2",
                    params![config, id.as_str()],
                )?;
            }
            let updated = provider_by_id(&transaction, &id)?;
            transaction.commit()?;
            Ok(updated)
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

    /// The entry's accounts, ordered by id.
    pub async fn credentials(&self, provider: ProviderId) -> Result<Vec<CredentialRow>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {CREDENTIAL_COLUMNS} FROM provider_credentials WHERE provider_id = ?1 ORDER BY id"
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
    /// one transaction; `false` for an entry that no longer exists.
    pub async fn write_credential(
        &self,
        provider: ProviderId,
        account: CredentialWrite,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let exists: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM providers WHERE id = ?1)",
                [provider.as_str()],
                |row| row.get(0),
            )?;
            if !exists {
                return Ok(false);
            }
            write_credential(&transaction, &provider, &account, now)?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Stores a refreshed secret only while the account is still at
    /// `version`; `false` when another writer stored first.
    pub async fn replace_credential_secret(
        &self,
        provider: ProviderId,
        id: CredentialId,
        secret: Vec<u8>,
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

    /// Removes the account, and the entry's selection of it with it.
    pub async fn remove_credential(&self, provider: ProviderId, id: CredentialId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM provider_credentials WHERE provider_id = ?1 AND id = ?2",
                params![provider.as_str(), id.as_str()],
            )?;
            transaction.execute(
                "UPDATE providers SET active_credential_id = NULL WHERE id = ?1 AND active_credential_id = ?2",
                params![provider.as_str(), id.as_str()],
            )?;
            transaction.commit()?;
            Ok(())
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
    pub async fn catalog_record(&self, provider: ProviderId) -> Result<Option<CatalogRecord>, StorageError> {
        self.call(move |connection, _| {
            let text: Option<String> = connection
                .query_row(
                    "SELECT record FROM model_catalogs WHERE provider_id = ?1",
                    [provider.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            text.map(|text| json("model_catalogs", "record", &text)).transpose()
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
            connection.execute("DELETE FROM model_catalogs WHERE provider_id = ?1", [provider.as_str()])?;
            Ok(())
        })
        .await
    }
}

fn provider_by_id(connection: &Connection, id: &ProviderId) -> Result<Option<ProviderRow>, StorageError> {
    let mut statement =
        connection.prepare_cached(&format!("SELECT {PROVIDER_COLUMNS} FROM providers WHERE id = ?1"))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(provider_row).transpose()
}

/// Inserts or replaces an account, advancing its version, and selects it
/// when the entry has no active account.
fn write_credential(
    connection: &Connection,
    provider: &ProviderId,
    account: &CredentialWrite,
    now: Timestamp,
) -> Result<(), StorageError> {
    connection.execute(
        "INSERT INTO provider_credentials
           (provider_id, id, identity_key, label, detail, source, secret, version, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)
         ON CONFLICT (provider_id, id) DO UPDATE SET
           identity_key = excluded.identity_key, label = excluded.label, detail = excluded.detail,
           source = excluded.source, secret = excluded.secret, version = version + 1,
           updated_at = excluded.updated_at",
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
    connection.execute(
        "UPDATE providers SET active_credential_id = ?2 WHERE id = ?1 AND active_credential_id IS NULL",
        params![provider.as_str(), account.id.as_str()],
    )?;
    Ok(())
}

fn provider_row(row: &Row<'_>) -> Result<ProviderRow, StorageError> {
    let active: Option<String> = row.get("active_credential_id")?;
    Ok(ProviderRow {
        id: decode("providers", "id", ProviderId::try_from(row.get::<_, String>("id")?))?,
        owner: decode(
            "providers",
            "owner_user_id",
            UserId::try_from(row.get::<_, String>("owner_user_id")?),
        )?,
        family: row.get("provider_type")?,
        kind: decode(
            "providers",
            "credential_kind",
            row.get::<_, String>("credential_kind")?.parse::<CredentialKind>(),
        )?,
        label: row.get("label")?,
        config: row.get("config")?,
        active: active
            .map(|id| decode("providers", "active_credential_id", CredentialId::try_from(id)))
            .transpose()?,
        created_at: instant(row, "providers", "created_at")?,
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
    })
}
