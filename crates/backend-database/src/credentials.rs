//! The users' credentials (`credentials.md` § Storage and expiry,
//! `storage.md` § Passwords and credentials at rest): each credential's
//! name, note, fields and times in the clear, and its values sealed with the
//! user and the name as the additional data, so a rename seals them again in
//! the same transaction; and the conversations allowed to use each. The
//! control database stores the values only sealed; whoever calls seals and
//! opens them through [`CredentialSeal`], on the database's thread, so that
//! a change that reads and writes values does so in one transaction.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use demi_shared_types::Timestamp;
use demi_web_api_protocol::credentials::{CredentialDto, CredentialField, FieldKind, Keep, ListedField};
use demi_web_api_protocol::ids::{ConversationId, UserCredentialId, UserId};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;

pub(crate) const TABLE: &str = "user_credentials";

pub(crate) const COLUMNS: &str =
    "id, user_id, name, note, fields, keep_until, last_used_at, created_at, updated_at";

/// What seals and opens a credential's values: the product's one owner of
/// sealing, for the credential `name` of `user`.
pub trait CredentialSeal: Send + Sync {
    fn seal(&self, user: &UserId, name: &str, plaintext: &[u8]) -> Vec<u8>;
    /// What `sealed` holds; none when it does not open.
    fn open(&self, user: &UserId, name: &str, sealed: &[u8]) -> Option<Vec<u8>>;
}

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
    pub user: UserId,
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

/// A new credential.
#[derive(Debug, Clone)]
pub struct NewCredential {
    pub name: String,
    pub note: String,
    pub fields: Vec<CredentialField>,
    pub values: BTreeMap<String, String>,
    pub keep_until: Option<Timestamp>,
}

/// A change of a credential: what it names changes.
#[derive(Debug, Clone, Default)]
pub struct CredentialChange {
    pub name: Option<String>,
    pub note: Option<String>,
    /// The whole list; a field without a value keeps the one it has.
    pub fields: Option<Vec<(CredentialField, Option<String>)>>,
    /// The new removal time, none for always.
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
    /// A new field, by its label, sent without a value.
    #[error("Enter a value for the new field \u{201c}{0}\u{201d}.")]
    MissingValue(String),
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

impl ControlService {
    /// The user's credentials as Settings lists them, in name order: each
    /// visible field with its value, opened through `seal`.
    pub async fn user_credential_list(
        &self,
        user: UserId,
        seal: Arc<dyn CredentialSeal>,
    ) -> Result<Vec<CredentialDto>, StorageError> {
        self.call(move |connection, _| {
            let stored = connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM user_credentials WHERE user_id = ?1 ORDER BY name, id"
                ))?
                .query_map([user.as_str()], |row| Ok(credential_row(row)))?
                .map(|row| row?)
                .collect::<Result<Vec<_>, StorageError>>()?;
            stored
                .iter()
                .map(|credential| Ok(credential.dto(&values_of(connection, credential, &*seal)?)))
                .collect()
        })
        .await
    }

    /// The user's credentials, in name order.
    pub async fn user_credentials(&self, user: UserId) -> Result<Vec<StoredCredential>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM user_credentials WHERE user_id = ?1 ORDER BY name, id"
                ))?
                .query_map([user.as_str()], |row| Ok(credential_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// The user's credential named `name`.
    pub async fn user_credential_named(
        &self,
        user: UserId,
        name: String,
    ) -> Result<Option<StoredCredential>, StorageError> {
        self.call(move |connection, _| named(connection, &user, &name)).await
    }

    /// The ids of the credentials the conversation may use.
    pub async fn allowed_credentials(
        &self,
        conversation: ConversationId,
    ) -> Result<BTreeSet<UserCredentialId>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(
                    "SELECT credential_id FROM credential_allowances WHERE conversation_id = ?1",
                )?
                .query_map([conversation.as_str()], |row| row.get::<_, String>(0))?
                .map(|id| decode(
                    "credential_allowances",
                    "credential_id",
                    UserCredentialId::try_from(id?),
                ))
                .collect()
        })
        .await
    }

    /// The user's credential `id` with every value, opened.
    pub async fn user_credential_values(
        &self,
        user: UserId,
        id: UserCredentialId,
        seal: Arc<dyn CredentialSeal>,
    ) -> Result<Option<(StoredCredential, BTreeMap<String, String>)>, StorageError> {
        self.call(move |connection, _| {
            let Some(stored) = by_id(connection, &user, &id)? else {
                return Ok(None);
            };
            let values = values_of(connection, &stored, &*seal)?;
            Ok(Some((stored, values)))
        })
        .await
    }

    /// Saves a new credential, unless the user has one of its name, and
    /// answers it as Settings lists it.
    pub async fn create_user_credential(
        &self,
        user: UserId,
        new: NewCredential,
        seal: Arc<dyn CredentialSeal>,
    ) -> Result<Result<Saved, CredentialRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if let Some(other) = named(&transaction, &user, &new.name)? {
                return Ok(Err(CredentialRefusal::NameTaken(other.name)));
            }
            let id = insert(&transaction, &user, &new, now, &*seal)?;
            let stored = by_id(&transaction, &user, &id)?.expect("the credential was just written");
            let conversations = waiting_for(&transaction, &user, &[&new.name])?;
            transaction.commit()?;
            Ok(Ok(Saved {
                credential: stored.dto(&new.values),
                conversations: conversations.into_iter().collect(),
            }))
        })
        .await
    }

    /// Changes the user's credential `id`, sealing its values again under
    /// its name, in one transaction, and answers it as Settings lists it.
    pub async fn change_user_credential(
        &self,
        user: UserId,
        id: UserCredentialId,
        change: CredentialChange,
        seal: Arc<dyn CredentialSeal>,
    ) -> Result<Result<Saved, CredentialRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let Some(stored) = by_id(&transaction, &user, &id)? else {
                return Ok(Err(CredentialRefusal::NotFound));
            };
            let name = change.name.unwrap_or_else(|| stored.name.clone());
            // A change of the name's case alone keeps it this credential's.
            if let Some(other) = named(&transaction, &user, &name)?.filter(|other| other.id != id) {
                return Ok(Err(CredentialRefusal::NameTaken(other.name)));
            }
            let old = values_of(&transaction, &stored, &*seal)?;
            let (fields, values) = match change.fields {
                None => (stored.fields.clone(), old),
                Some(fields) => {
                    let mut values = BTreeMap::new();
                    let mut kept = Vec::with_capacity(fields.len());
                    for (field, value) in fields {
                        let value = match value.or_else(|| old.get(&field.name).cloned()) {
                            Some(value) => value,
                            None => {
                                return Ok(Err(CredentialRefusal::MissingValue(
                                    field.label.into_string(),
                                )));
                            }
                        };
                        values.insert(field.name.clone(), value);
                        kept.push(field);
                    }
                    (kept, values)
                }
            };
            let note = change.note.unwrap_or_else(|| stored.note.clone());
            let keep_until = change.keep_until.unwrap_or(stored.keep_until);
            let sealed = seal.seal(&user, &name, to_json(&values).as_bytes());
            transaction.execute(
                "UPDATE user_credentials
                 SET name = ?3, note = ?4, fields = ?5, sealed = ?6, keep_until = ?7, updated_at = ?8
                 WHERE id = ?1 AND user_id = ?2",
                params![
                    id.as_str(),
                    user.as_str(),
                    name,
                    note,
                    to_json(&fields),
                    sealed,
                    keep_until.map(Timestamp::as_millisecond),
                    now.as_millisecond()
                ],
            )?;
            let changed = by_id(&transaction, &user, &id)?.expect("the credential was just written");
            // A rename changes the requests of the old name and of the new one.
            let conversations = waiting_for(&transaction, &user, &[&stored.name, &changed.name])?;
            transaction.commit()?;
            Ok(Ok(Saved {
                credential: changed.dto(&values),
                conversations: conversations.into_iter().collect(),
            }))
        })
        .await
    }

    /// Removes the user's credential `id` and its allowances; an
    /// allowance request that named it then shows it removed, and a waiting
    /// credential request of its name no longer renews it. Answers the
    /// conversations of those requests, or none when the user has no such
    /// credential.
    pub async fn remove_user_credential(
        &self,
        user: UserId,
        id: UserCredentialId,
    ) -> Result<Option<Vec<ConversationId>>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(stored) = by_id(&transaction, &user, &id)? else {
                return Ok(None);
            };
            let mut conversations = requesting(&transaction, &[id.clone()])?;
            conversations.extend(waiting_for(&transaction, &user, &[&stored.name])?);
            transaction.execute(
                "DELETE FROM user_credentials WHERE id = ?1 AND user_id = ?2",
                params![id.as_str(), user.as_str()],
            )?;
            transaction.commit()?;
            Ok(Some(conversations.into_iter().collect()))
        })
        .await
    }

    /// Removes the user's credentials whose time has come, and answers when
    /// the next one's comes (`credentials.md` § Storage and expiry).
    pub async fn expire_user_credentials(&self, user: UserId) -> Result<Expired, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let (due, names): (Vec<UserCredentialId>, Vec<String>) = transaction
                .prepare_cached(
                    "SELECT id, name FROM user_credentials WHERE user_id = ?1 AND keep_until <= ?2",
                )?
                .query_map(params![user.as_str(), now.as_millisecond()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .map(|row| {
                    let (id, name) = row?;
                    Ok((decode(TABLE, "id", UserCredentialId::try_from(id))?, name))
                })
                .collect::<Result<Vec<_>, StorageError>>()?
                .into_iter()
                .unzip();
            let mut conversations = requesting(&transaction, &due)?;
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            conversations.extend(waiting_for(&transaction, &user, &names)?);
            for id in &due {
                transaction.execute("DELETE FROM user_credentials WHERE id = ?1", [id.as_str()])?;
            }
            let next: Option<i64> = transaction.query_row(
                "SELECT min(keep_until) FROM user_credentials WHERE user_id = ?1",
                [user.as_str()],
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
    user: &UserId,
    names: &[&str],
) -> Result<BTreeSet<ConversationId>, StorageError> {
    let wanted: BTreeSet<String> = names.iter().map(|name| name.to_lowercase()).collect();
    let mut conversations = BTreeSet::new();
    let mut statement = connection.prepare_cached(
        "SELECT r.conversation_id, r.credential_name FROM permission_requests r
         JOIN conversations c ON c.id = r.conversation_id
         WHERE c.user_id = ?1 AND r.kind = 'credential' AND r.decision IS NULL",
    )?;
    let mut rows = statement.query([user.as_str()])?;
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

/// The conversations whose requests name one of `credentials`.
fn requesting(
    connection: &Connection,
    credentials: &[UserCredentialId],
) -> Result<BTreeSet<ConversationId>, StorageError> {
    let mut conversations = BTreeSet::new();
    for id in credentials {
        let found: Vec<String> = connection
            .prepare_cached(
                "SELECT DISTINCT conversation_id FROM permission_requests WHERE credential_id = ?1",
            )?
            .query_map([id.as_str()], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for conversation in found {
            conversations.insert(decode(
                "permission_requests",
                "conversation_id",
                ConversationId::try_from(conversation.as_str()),
            )?);
        }
    }
    Ok(conversations)
}

/// Writes a new credential and answers its id.
pub(crate) fn insert(
    connection: &Connection,
    user: &UserId,
    new: &NewCredential,
    now: Timestamp,
    seal: &dyn CredentialSeal,
) -> Result<UserCredentialId, StorageError> {
    let id = UserCredentialId::try_from(uuid::Uuid::new_v4().to_string())
        .expect("a UUID is not empty");
    let sealed = seal.seal(user, &new.name, to_json(&new.values).as_bytes());
    connection.execute(
        "INSERT INTO user_credentials
         (id, user_id, name, note, fields, sealed, keep_until, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id.as_str(),
            user.as_str(),
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
    stored: &StoredCredential,
    values: &BTreeMap<String, String>,
    keep_until: Option<Timestamp>,
    now: Timestamp,
    seal: &dyn CredentialSeal,
) -> Result<(), StorageError> {
    let sealed = seal.seal(&stored.user, &stored.name, to_json(values).as_bytes());
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
    stored: &StoredCredential,
    seal: &dyn CredentialSeal,
) -> Result<BTreeMap<String, String>, StorageError> {
    let sealed: Vec<u8> = connection.query_row(
        "SELECT sealed FROM user_credentials WHERE id = ?1",
        [stored.id.as_str()],
        |row| row.get(0),
    )?;
    let opened = seal
        .open(&stored.user, &stored.name, &sealed)
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
    user: &UserId,
    id: &UserCredentialId,
) -> Result<Option<StoredCredential>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM user_credentials WHERE id = ?1 AND user_id = ?2"),
            params![id.as_str(), user.as_str()],
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
    user: &UserId,
    name: &str,
) -> Result<Option<StoredCredential>, StorageError> {
    let wanted = name.to_lowercase();
    let mut statement = connection
        .prepare_cached(&format!("SELECT {COLUMNS} FROM user_credentials WHERE user_id = ?1"))?;
    let mut rows = statement.query([user.as_str()])?;
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
        user: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        name: row.get("name")?,
        note: row.get("note")?,
        fields,
        keep_until: optional_instant(row, "keep_until")?,
        last_used_at: optional_instant(row, "last_used_at")?,
        created_at: instant(row, TABLE, "created_at")?,
        updated_at: instant(row, TABLE, "updated_at")?,
    })
}
