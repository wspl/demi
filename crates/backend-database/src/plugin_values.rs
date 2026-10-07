//! The users' plugin choices and plugin values (`storage.md` § Control
//! records; `plugins.md` § A user's plugins, § The contract): whether a user
//! has each plugin on, a plugin with no row being on; each plugin's JSON
//! documents for a user, by key, each with the revision a conditional write
//! names and the blobs it names; and each plugin's Host directories for a
//! user. The control database decodes a document only as JSON; its plugin
//! decodes it into its own type.

use std::collections::BTreeMap;

use demi_plugin_interface::{DirectoryFile, HostDirectory};
use demi_shared_types::BlobRef;
use demi_web_api_protocol::ids::UserId;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

const TABLE: &str = "plugin_values";
const DIRECTORIES: &str = "plugin_directories";

/// A stored value and its revision.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginValue {
    pub document: Value,
    pub revision: u64,
}

impl ControlService {
    /// Each plugin `user` turned on or off, by id; one the user never
    /// switched is on.
    pub async fn user_plugins(&self, user: UserId) -> Result<BTreeMap<String, bool>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection
                .prepare_cached("SELECT plugin, enabled FROM user_plugins WHERE user_id = ?1")?;
            let mut rows = statement.query([user.as_str()])?;
            let mut choices = BTreeMap::new();
            while let Some(row) = rows.next()? {
                choices.insert(row.get::<_, String>(0)?, row.get::<_, bool>(1)?);
            }
            Ok(choices)
        })
        .await
    }

    /// Records that `user` has `plugin` on or off.
    pub async fn set_user_plugin(
        &self,
        user: UserId,
        plugin: String,
        enabled: bool,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO user_plugins (user_id, plugin, enabled) VALUES (?1, ?2, ?3)
                 ON CONFLICT (user_id, plugin) DO UPDATE SET enabled = excluded.enabled",
                params![user.as_str(), plugin, enabled],
            )?;
            Ok(())
        })
        .await
    }

    /// `plugin`'s value `key` for `user`.
    pub async fn plugin_value(
        &self,
        user: UserId,
        plugin: String,
        key: String,
    ) -> Result<Option<PluginValue>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT document, revision FROM plugin_values
                 WHERE user_id = ?1 AND plugin = ?2 AND key = ?3",
            )?;
            let row = statement
                .query_row(params![user.as_str(), plugin, key], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .optional()?;
            row.map(|(document, revision)| plugin_value(&document, revision))
                .transpose()
        })
        .await
    }

    /// Every value of `plugin`'s for `user`, by key.
    pub async fn plugin_values(
        &self,
        user: UserId,
        plugin: String,
    ) -> Result<BTreeMap<String, PluginValue>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT key, document, revision FROM plugin_values
                 WHERE user_id = ?1 AND plugin = ?2",
            )?;
            let mut rows = statement.query(params![user.as_str(), plugin])?;
            let mut values = BTreeMap::new();
            while let Some(row) = rows.next()? {
                let key: String = row.get(0)?;
                let value = plugin_value(&row.get::<_, String>(1)?, row.get(2)?)?;
                values.insert(key, value);
            }
            Ok(values)
        })
        .await
    }

    /// Writes `plugin`'s value `key` for `user` if it is still at
    /// `revision`, none for a value that does not exist yet, in one
    /// transaction, so two writes never build on the same revision. The
    /// value names `blobs`.
    pub async fn write_plugin_value(&self, write: ValueWrite) -> Result<Written, StorageError> {
        let text = write.document.to_string();
        let named = blob_list(&write.blobs);
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let before: Option<i64> = transaction
                .query_row(
                    "SELECT revision FROM plugin_values
                     WHERE user_id = ?1 AND plugin = ?2 AND key = ?3",
                    params![write.user.as_str(), write.plugin, write.key],
                    |row| row.get(0),
                )
                .optional()?;
            let stored = before
                .map(|revision| decode(TABLE, "revision", u64::try_from(revision)))
                .transpose()?;
            if stored != write.revision {
                return Ok(Written::Conflict);
            }
            let revision = write.revision.map_or(1, |revision| revision + 1);
            transaction.execute(
                "INSERT INTO plugin_values (user_id, plugin, key, document, revision, blobs)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (user_id, plugin, key) DO UPDATE SET
                   document = excluded.document,
                   revision = excluded.revision,
                   blobs = excluded.blobs",
                params![
                    write.user.as_str(),
                    write.plugin,
                    write.key,
                    text,
                    i64::try_from(revision).unwrap_or(i64::MAX),
                    named,
                ],
            )?;
            transaction.commit()?;
            Ok(Written::Revision(revision))
        })
        .await
    }

    /// Removes `plugin`'s value `key` for `user` if it is still at
    /// `revision`, in one transaction. Answers [`Written::Revision`] with
    /// the removed revision.
    pub async fn remove_plugin_value(
        &self,
        user: UserId,
        plugin: String,
        key: String,
        revision: u64,
    ) -> Result<Written, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let before: Option<i64> = transaction
                .query_row(
                    "SELECT revision FROM plugin_values
                     WHERE user_id = ?1 AND plugin = ?2 AND key = ?3",
                    params![user.as_str(), plugin, key],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(stored) = before else {
                return Ok(Written::Conflict);
            };
            if decode(TABLE, "revision", u64::try_from(stored))? != revision {
                return Ok(Written::Conflict);
            }
            transaction.execute(
                "DELETE FROM plugin_values WHERE user_id = ?1 AND plugin = ?2 AND key = ?3",
                params![user.as_str(), plugin, key],
            )?;
            transaction.commit()?;
            Ok(Written::Revision(revision))
        })
        .await
    }

    /// Every Host directory of each of `user`'s plugins, by plugin id, each
    /// set in name order.
    pub async fn plugin_directories(
        &self,
        user: UserId,
    ) -> Result<BTreeMap<String, Vec<HostDirectory>>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT plugin, name, files FROM plugin_directories
                 WHERE user_id = ?1 ORDER BY plugin, name",
            )?;
            let mut rows = statement.query([user.as_str()])?;
            let mut sets: BTreeMap<String, Vec<HostDirectory>> = BTreeMap::new();
            while let Some(row) = rows.next()? {
                let files: String = row.get(2)?;
                let directory = HostDirectory {
                    name: row.get(1)?,
                    files: decode(DIRECTORIES, "files", serde_json::from_str(&files))?,
                };
                sets.entry(row.get(0)?).or_default().push(directory);
            }
            Ok(sets)
        })
        .await
    }

    /// Replaces `plugin`'s Host directories for `user` whole, in one
    /// transaction.
    pub async fn set_plugin_directories(
        &self,
        user: UserId,
        plugin: String,
        directories: Vec<HostDirectory>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM plugin_directories WHERE user_id = ?1 AND plugin = ?2",
                params![user.as_str(), plugin],
            )?;
            for directory in &directories {
                let files = serde_json::to_string(&directory.files)
                    .expect("a directory's files encode as JSON");
                transaction.execute(
                    "INSERT INTO plugin_directories (user_id, plugin, name, digest, files)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        user.as_str(),
                        plugin,
                        directory.name,
                        directory.digest(),
                        files
                    ],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }
}

/// A write of a plugin value.
pub struct ValueWrite {
    pub user: UserId,
    pub plugin: String,
    pub key: String,
    pub document: Value,
    /// The revision the write read; none for a value that does not exist
    /// yet.
    pub revision: Option<u64>,
    /// The blobs the value names.
    pub blobs: Vec<BlobRef>,
}

/// What a write of a plugin value did.
#[derive(Debug)]
pub enum Written {
    /// The value's new revision.
    Revision(u64),
    /// Another write came first.
    Conflict,
}

/// The blobs `user`'s plugins name: each value's list, and the files of each
/// Host directory.
pub(crate) fn plugin_blobs(
    connection: &Connection,
    user: &UserId,
) -> Result<Vec<BlobRef>, StorageError> {
    let mut blobs = Vec::new();
    let mut values = connection.prepare_cached("SELECT blobs FROM plugin_values WHERE user_id = ?1")?;
    let mut rows = values.query([user.as_str()])?;
    while let Some(row) = rows.next()? {
        let named: Vec<BlobRef> =
            decode(TABLE, "blobs", serde_json::from_str(&row.get::<_, String>(0)?))?;
        blobs.extend(named);
    }
    let mut directories =
        connection.prepare_cached("SELECT files FROM plugin_directories WHERE user_id = ?1")?;
    let mut rows = directories.query([user.as_str()])?;
    while let Some(row) = rows.next()? {
        let files: Vec<DirectoryFile> =
            decode(DIRECTORIES, "files", serde_json::from_str(&row.get::<_, String>(0)?))?;
        blobs.extend(files.into_iter().map(|file| file.blob));
    }
    Ok(blobs)
}

/// `blobs` as their column holds them.
fn blob_list(blobs: &[BlobRef]) -> String {
    let names: Vec<&str> = blobs.iter().map(BlobRef::as_str).collect();
    serde_json::to_string(&names).expect("names encode as JSON")
}

fn plugin_value(document: &str, revision: i64) -> Result<PluginValue, StorageError> {
    Ok(PluginValue {
        document: decode(TABLE, "document", serde_json::from_str(document))?,
        revision: decode(TABLE, "revision", u64::try_from(revision))?,
    })
}
