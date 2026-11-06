//! The users' credentials (`crates/backend/src/config/secret.rs`).
//!
//! The module stores:
//!
//! - each credential's name, note, fields and times in the clear;
//! - its values, sealed with the user and the name as the additional data, so a
//!   rename seals them again in the same transaction;
//! - the conversations allowed to use each.
//!
//! The core's database holds the values only sealed, under the core's key.
//! The store seals and opens them on the database's thread, so that a change
//! that reads and writes values does so in one transaction.

use std::collections::{BTreeMap, BTreeSet};

use demi_shared_types::Timestamp;
use demi_web_api_protocol::credentials::{CredentialDto, CredentialField, FieldKind, Keep, ListedField};
use demi_web_api_protocol::ids::{ConversationId, UserCredentialId, UserId};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::seal::{self, VaultKey};
use super::store::CoreStore;

pub(crate) const TABLE: &str = "user_credentials";

pub(crate) const COLUMNS: &str =
    "id, name, note, fields, keep_until, last_used_at, created_at, updated_at";

/// A credential's fields as their column holds them: credential fields
/// format 1.
#[derive(Debug, Serialize, Deserialize, Validate)]
#[serde(transparent)]
#[garde(transparent)]
pub(crate) struct Fields(#[garde(length(min = 1), dive)] pub(crate) Vec<CredentialField>);

/// A credential's values by field name, as they are sealed.
#[derive(Debug, Serialize, Deserialize, Validate)]
#[serde(transparent)]
struct Values(#[garde(skip)] BTreeMap<String, String>);

/// A credential as it is stored, without its values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCredential {
    pub id: UserCredentialId,
    pub name: String,
    pub note: String,
    pub fields: Vec<CredentialField>,
    /// None while it is kept always.
    pub keep_until: Option<Timestamp>,
    pub last_used_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl StoredCredential {
    /// The credential as Settings lists it, its visible fields with their
    /// values among `values` and its secret ones without.
    pub fn dto(&self, values: &BTreeMap<String, String>) -> CredentialDto {
        CredentialDto {
            id: self.id.clone(),
            name: self.name.clone(),
            note: self.note.clone(),
            fields: self
                .fields
                .iter()
                .map(|field| ListedField {
                    name: field.name.clone(),
                    label: field.label.as_str().to_owned(),
                    kind: field.kind,
                    value: match field.kind {
                        FieldKind::Visible => values.get(&field.name).cloned(),
                        FieldKind::Secret => None,
                    },
                    missing: !values.contains_key(&field.name),
                })
                .collect(),
            keep_until: self.keep_until,
            last_used_at: self.last_used_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    /// Whether `fields` are this credential's: the same names, each of the
    /// same kind, whatever their labels and order.
    pub fn same_fields(&self, fields: &[CredentialField]) -> bool {
        shape(&self.fields) == shape(fields)
    }
}

fn shape(fields: &[CredentialField]) -> BTreeSet<(&str, &'static str)> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field.kind.as_str()))
        .collect()
}

/// A new credential. A field `values` does not name is missing.
#[derive(Debug, Clone)]
pub struct NewCredential {
    pub name: String,
    pub note: String,
    pub fields: Vec<CredentialField>,
    pub values: BTreeMap<String, String>,
    pub keep_until: Option<Timestamp>,
}

/// A change of a credential: an absent member stays as it is.
#[derive(Debug, Clone, Default)]
pub struct CredentialChange {
    pub name: Option<String>,
    pub note: Option<String>,
    /// The fields as they now are; a field that keeps its name keeps its
    /// value, and one that goes takes its value with it.
    pub fields: Option<Vec<CredentialField>>,
    /// A new value for each field named; an empty one leaves it missing.
    pub values: BTreeMap<String, String>,
    /// The new removal time, none for always; absent, it stays.
    pub keep_until: Option<Option<Timestamp>>,
}

/// Why a credential was not saved or changed, in words for the user, who
/// reads them as they are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialRefusal {
    #[error("This credential no longer exists.")]
    NotFound,
    /// The name of the other credential, as it is saved.
    #[error("Another credential is named \u{201c}{0}\u{201d}. Choose another name.")]
    NameTaken(String),
}

/// What the expiry of a user's credentials removed, and what is next.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expired {
    /// Whether it removed any.
    pub removed: bool,
    /// The conversations whose requests named one it removed, by its id or
    /// by its name.
    pub conversations: Vec<ConversationId>,
    /// When the user's next credential is removed.
    pub next: Option<Timestamp>,
}

/// A credential as a save or a change of it left it, with the conversations
/// whose requests it changed: those with a credential request of its name,
/// or of the name it had, waiting, whose renewal follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub credential: CredentialDto,
    pub conversations: Vec<ConversationId>,
}

impl CoreStore {
    /// The user's credentials as Settings lists them, in name order: each
    /// visible field with its value, opened.
    pub async fn user_credential_list(
        &self,
    ) -> Result<Vec<CredentialDto>, StorageError> {
        let principal = self.principal().clone();
        let key = self.key().clone();
        self.call(move |connection, _| {
            let stored = connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM user_credentials ORDER BY name, id"
                ))?
                .query_map([], |row| Ok(credential_row(row)))?
                .map(|row| row?)
                .collect::<Result<Vec<_>, StorageError>>()?;
            stored
                .iter()
                .map(|credential| Ok(credential.dto(&values_of(connection, &principal, credential, &key)?)))
                .collect()
        })
        .await
    }

    /// The user's credentials, in name order.
    pub async fn user_credentials(&self) -> Result<Vec<StoredCredential>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM user_credentials ORDER BY name, id"
                ))?
                .query_map([], |row| Ok(credential_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The user's credential named `name`.
    pub async fn user_credential_named(
        &self,
        name: String,
    ) -> Result<Option<StoredCredential>, StorageError> {
        self.call(move |connection, _| named(connection, &name)).await
    }

    /// The user's credential `id` with every value, opened.
    pub async fn user_credential_values(
        &self,
        id: UserCredentialId,
    ) -> Result<Option<(StoredCredential, BTreeMap<String, String>)>, StorageError> {
        let principal = self.principal().clone();
        let key = self.key().clone();
        self.call(move |connection, _| {
            let Some(stored) = by_id(connection, &id)? else {
                return Ok(None);
            };
            let values = values_of(connection, &principal, &stored, &key)?;
            Ok(Some((stored, values)))
        })
        .await
    }

    /// Saves a new credential, unless the user has one of its name, and
    /// answers it as Settings lists it.
    pub async fn create_user_credential(
        &self,
        new: NewCredential,
    ) -> Result<Result<Saved, CredentialRefusal>, StorageError> {
        let principal = self.principal().clone();
        let key = self.key().clone();
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if let Some(other) = named(&transaction, &new.name)? {
                return Ok(Err(CredentialRefusal::NameTaken(other.name)));
            }
            let id = insert(&transaction, &principal, &new, now, &key)?;
            let stored = by_id(&transaction, &id)?.expect("the credential was just written");
            let conversations = waiting_for(&transaction, &[&new.name])?;
            transaction.commit()?;
            Ok(Ok(Saved {
                credential: stored.dto(&new.values),
                conversations: conversations.into_iter().collect(),
            }))
        })
        .await
    }

    /// Changes the user's credential `id` as `change` says, unless another of
    /// theirs has its new name, and answers it as Settings lists it. Its
    /// values are sealed again under its name, as it now is.
    pub async fn change_user_credential(
        &self,
        id: UserCredentialId,
        change: CredentialChange,
    ) -> Result<Result<Saved, CredentialRefusal>, StorageError> {
        let principal = self.principal().clone();
        let key = self.key().clone();
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let Some(stored) = by_id(&transaction, &id)? else {
                return Ok(Err(CredentialRefusal::NotFound));
            };
            if let Some(name) = &change.name
                && let Some(other) = named(&transaction, name)?
                && other.id != stored.id
            {
                return Ok(Err(CredentialRefusal::NameTaken(other.name)));
            }
            let mut values = values_of(&transaction, &principal, &stored, &key)?;
            let fields = change.fields.unwrap_or_else(|| stored.fields.clone());
            // A field keeps its value while it keeps its name and its kind: a
            // secret made visible starts missing, so no list ever shows it.
            values.retain(|name, _| {
                let was = stored.fields.iter().find(|field| &field.name == name);
                fields
                    .iter()
                    .any(|field| &field.name == name && was.is_some_and(|was| was.kind == field.kind))
            });
            for (name, value) in change.values {
                if !fields.iter().any(|field| field.name == name) {
                    continue;
                }
                if value.is_empty() {
                    values.remove(&name);
                } else {
                    values.insert(name, value);
                }
            }
            let name = change.name.unwrap_or_else(|| stored.name.clone());
            let note = change.note.unwrap_or_else(|| stored.note.clone());
            let keep_until = change.keep_until.unwrap_or(stored.keep_until);
            let sealed = key.seal(seal::Row::Credential { user: &principal, name: &name }, to_json(&values).as_bytes());
            transaction.execute(
                "UPDATE user_credentials
                 SET name = ?2, note = ?3, fields = ?4, sealed = ?5, keep_until = ?6, updated_at = ?7
                 WHERE id = ?1",
                params![
                    stored.id.as_str(),
                    name,
                    note,
                    to_json(&fields),
                    sealed,
                    keep_until.map(Timestamp::as_millisecond),
                    now.as_millisecond()
                ],
            )?;
            let changed = by_id(&transaction, &id)?.expect("the credential was just written");
            let conversations = waiting_for(&transaction, &[&stored.name, &name])?;
            transaction.commit()?;
            Ok(Ok(Saved {
                credential: changed.dto(&values),
                conversations: conversations.into_iter().collect(),
            }))
        })
        .await
    }

    /// Removes the user's credential `id`.
    ///
    /// A waiting credential request of its name no longer renews it. Answers
    /// the conversations of those requests, or none when the user has no such
    /// credential.
    pub async fn remove_user_credential(
        &self,
        id: UserCredentialId,
    ) -> Result<Option<Vec<ConversationId>>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(stored) = by_id(&transaction, &id)? else {
                return Ok(None);
            };
            let conversations = waiting_for(&transaction, &[&stored.name])?;
            transaction.execute(
                "DELETE FROM user_credentials WHERE id = ?1",
                params![id.as_str()],
            )?;
            transaction.commit()?;
            Ok(Some(conversations.into_iter().collect()))
        })
        .await
    }

    /// Removes the user's credentials whose time has come, and answers when
    /// the next one's comes.
    pub async fn expire_user_credentials(&self) -> Result<Expired, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let (due, names): (Vec<UserCredentialId>, Vec<String>) = transaction
                .prepare_cached(
                    "SELECT id, name FROM user_credentials WHERE keep_until <= ?1",
                )?
                .query_map(params![now.as_millisecond()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .map(|row| {
                    let (id, name) = row?;
                    Ok((decode(TABLE, "id", UserCredentialId::try_from(id))?, name))
                })
                .collect::<Result<Vec<_>, StorageError>>()?
                .into_iter()
                .unzip();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let conversations = waiting_for(&transaction, &names)?;
            for id in &due {
                transaction.execute("DELETE FROM user_credentials WHERE id = ?1", [id.as_str()])?;
            }
            let next: Option<i64> = transaction.query_row(
                "SELECT min(keep_until) FROM user_credentials",
                [],
                |row| row.get(0),
            )?;
            transaction.commit()?;
            Ok(Expired {
                removed: !due.is_empty(),
                conversations: conversations.into_iter().collect(),
                next: next
                    .map(|next| decode(TABLE, "keep_until", Timestamp::from_millisecond(next)))
                    .transpose()?,
            })
        })
        .await
    }

    /// Removes every user's credentials whose time came while the backend
    /// was down, as it starts.
    pub async fn expire_all_user_credentials(&self) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                "DELETE FROM user_credentials WHERE keep_until <= ?1",
                [now.as_millisecond()],
            )?;
            Ok(())
        })
        .await
    }
}

/// When a credential kept as `keep` from `now` is removed; none for always.
pub fn keep_until(now: Timestamp, keep: Keep) -> Result<Option<Timestamp>, StorageError> {
    keep.duration()
        .map(|duration| {
            now.to_jiff()
                .checked_add(duration)
                .map(Timestamp::truncate)
                .map_err(StorageError::Time)
        })
        .transpose()
}

/// The user's conversations with a credential request waiting for one of
/// `names`, whatever its case.
pub(crate) fn waiting_for(
    connection: &Connection,
    names: &[&str],
) -> Result<BTreeSet<ConversationId>, StorageError> {
    let wanted: BTreeSet<String> = names.iter().map(|name| name.to_lowercase()).collect();
    let mut conversations = BTreeSet::new();
    let mut statement = connection.prepare_cached(
        "SELECT r.conversation_id, r.credential_name FROM permission_requests r
         JOIN conversations c ON c.id = r.conversation_id
         WHERE r.kind = 'credential' AND r.decision IS NULL",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: Option<String> = row.get(1)?;
        if name.is_some_and(|name| wanted.contains(&name.to_lowercase())) {
            conversations.insert(decode(
                "permission_requests",
                "conversation_id",
                ConversationId::try_from(row.get::<_, String>(0)?.as_str()),
            )?);
        }
    }
    Ok(conversations)
}

/// Writes a new credential and answers its id.
pub(crate) fn insert(
    connection: &Connection,
    principal: &UserId,
    new: &NewCredential,
    now: Timestamp,
    key: &VaultKey,
) -> Result<UserCredentialId, StorageError> {
    let id = UserCredentialId::try_from(uuid::Uuid::new_v4().to_string())
        .expect("a UUID is not empty");
    let sealed = key.seal(seal::Row::Credential { user: principal, name: &new.name }, to_json(&new.values).as_bytes());
    connection.execute(
        "INSERT INTO user_credentials
         (id, name, note, fields, sealed, keep_until, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            id.as_str(),
            
            new.name,
            new.note,
            to_json(&new.fields),
            sealed,
            new.keep_until.map(Timestamp::as_millisecond),
            now.as_millisecond()
        ],
    )?;
    Ok(id)
}

/// Gives the credential `id` new values and a new removal time, its fields
/// as they are.
pub(crate) fn renew(
    connection: &Connection,
    principal: &UserId,
    stored: &StoredCredential,
    values: &BTreeMap<String, String>,
    keep_until: Option<Timestamp>,
    now: Timestamp,
    key: &VaultKey,
) -> Result<(), StorageError> {
    let sealed = key.seal(seal::Row::Credential { user: principal, name: &stored.name }, to_json(values).as_bytes());
    connection.execute(
        "UPDATE user_credentials SET sealed = ?2, keep_until = ?3, updated_at = ?4 WHERE id = ?1",
        params![
            stored.id.as_str(),
            sealed,
            keep_until.map(Timestamp::as_millisecond),
            now.as_millisecond()
        ],
    )?;
    Ok(())
}

/// The credential's values, opened. A sealed value that does not open is
/// corrupt, and nothing replaces it.
pub(crate) fn values_of(
    connection: &Connection,
    principal: &UserId,
    stored: &StoredCredential,
    key: &VaultKey,
) -> Result<BTreeMap<String, String>, StorageError> {
    let sealed: Vec<u8> = connection.query_row(
        "SELECT sealed FROM user_credentials WHERE id = ?1",
        [stored.id.as_str()],
        |row| row.get(0),
    )?;
    let opened = key
        .open(seal::Row::Credential { user: principal, name: &stored.name }, &sealed)
        .ok()
        .and_then(|plaintext| String::from_utf8(plaintext).ok())
        .ok_or_else(|| StorageError::Corrupt {
            table: TABLE,
            column: "sealed",
            reason: format!("the values of the credential \u{201c}{}\u{201d} do not open", stored.name),
        })?;
    let Values(values) = json(TABLE, "sealed", &opened)?;
    Ok(values)
}

pub(crate) fn by_id(
    connection: &Connection,
    id: &UserCredentialId,
) -> Result<Option<StoredCredential>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM user_credentials WHERE id = ?1"),
            params![id.as_str()],
            |row| Ok(credential_row(row)),
        )
        .optional()?
        .transpose()
}

/// The user's credential named `name` whatever its case: *GitHub token*
/// and *github token* are one name. SQLite's own `NOCASE` folds ASCII alone, so the
/// names are compared here, among the user's few credentials.
pub(crate) fn named(
    connection: &Connection,
    name: &str,
) -> Result<Option<StoredCredential>, StorageError> {
    let wanted = name.to_lowercase();
    let mut statement = connection
        .prepare_cached(&format!("SELECT {COLUMNS} FROM user_credentials"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let stored = credential_row(row)?;
        if stored.name.to_lowercase() == wanted {
            return Ok(Some(stored));
        }
    }
    Ok(None)
}

fn optional_instant(
    row: &Row<'_>,
    column: &'static str,
) -> Result<Option<Timestamp>, StorageError> {
    let millis: Option<i64> = row.get(column)?;
    millis
        .map(|millis| decode(TABLE, column, Timestamp::from_millisecond(millis)))
        .transpose()
}

fn credential_row(row: &Row<'_>) -> Result<StoredCredential, StorageError> {
    let Fields(fields) = json(TABLE, "fields", &row.get::<_, String>("fields")?)?;
    Ok(StoredCredential {
        id: decode(TABLE, "id", UserCredentialId::try_from(row.get::<_, String>("id")?))?,
        name: row.get("name")?,
        note: row.get("note")?,
        fields,
        keep_until: optional_instant(row, "keep_until")?,
        last_used_at: optional_instant(row, "last_used_at")?,
        created_at: instant(row, TABLE, "created_at")?,
        updated_at: instant(row, TABLE, "updated_at")?,
    })
}
