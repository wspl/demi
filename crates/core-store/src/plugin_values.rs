//! The users' plugin choices and plugin values
//! (`crates/plugin-host/src/lib.rs`).
//!
//! The module stores:
//!
//! - whether a user has each plugin on, a plugin with no row being on, and
//!   each of its features, one with no row taking its default;
//! - each plugin's JSON documents for a user, by key, each with the revision a
//!   conditional write names and the blobs it names;
//! - each plugin's Host directories for a user.
//!
//! The control database decodes a document only as JSON; its plugin decodes it
//! into its own type.

use std::collections::BTreeMap;

use demi_plugin_interface::{DirectoryFile, HostDirectory};
use demi_plugin_host::store::{PluginValue, ValueWrite, Written};
use demi_shared_types::BlobRef;
use demi_web_api_protocol::ids::WorkspaceId;
use rusqlite::{Connection, OptionalExtension, params};

use super::StorageError;
use super::columns::decode;
use super::store::CoreStore;

const TABLE: &str = "plugin_values";
const DIRECTORIES: &str = "plugin_directories";

impl CoreStore {
    /// Each plugin `user` turned on or off, by id; one the user never
    /// switched is on.
    pub async fn user_plugins(&self) -> Result<BTreeMap<String, bool>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection
                .prepare_cached("SELECT plugin, enabled FROM user_plugins")?;
            let mut rows = statement.query([])?;
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
        plugin: String,
        enabled: bool,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO user_plugins (plugin, enabled) VALUES (?1, ?2)
                 ON CONFLICT (plugin) DO UPDATE SET enabled = excluded.enabled",
                params![plugin, enabled],
            )?;
            Ok(())
        })
        .await
    }

    /// Each plugin's feature `user` turned on or off, by the plugin's id and
    /// the feature's.
    pub async fn user_plugin_features(
        &self,
    ) -> Result<BTreeMap<(String, String), bool>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT plugin, feature, enabled FROM user_plugin_features",
            )?;
            let mut rows = statement.query([])?;
            let mut choices = BTreeMap::new();
            while let Some(row) = rows.next()? {
                choices.insert((row.get::<_, String>(0)?, row.get::<_, String>(1)?), row.get::<_, bool>(2)?);
            }
            Ok(choices)
        })
        .await
    }

    /// Records that `user` has `plugin`'s `feature` on or off.
    pub async fn set_user_plugin_feature(
        &self,
        plugin: String,
        feature: String,
        enabled: bool,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO user_plugin_features (plugin, feature, enabled) VALUES (?1, ?2, ?3)
                 ON CONFLICT (plugin, feature) DO UPDATE SET enabled = excluded.enabled",
                params![plugin, feature, enabled],
            )?;
            Ok(())
        })
        .await
    }

    /// `plugin`'s value `key` for `user`.
    pub async fn plugin_value(
        &self,
        plugin: String,
        key: String,
    ) -> Result<Option<PluginValue>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT document, revision FROM plugin_values
                 WHERE plugin = ?1 AND key = ?2",
            )?;
            let row = statement
                .query_row(params![plugin, key], |row| {
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
        plugin: String,
    ) -> Result<BTreeMap<String, PluginValue>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT key, document, revision FROM plugin_values
                 WHERE plugin = ?1",
            )?;
            let mut rows = statement.query(params![plugin])?;
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
                     WHERE plugin = ?1 AND key = ?2",
                    params![write.plugin, write.key],
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
                "INSERT INTO plugin_values (plugin, key, document, revision, blobs)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (plugin, key) DO UPDATE SET
                   document = excluded.document,
                   revision = excluded.revision,
                   blobs = excluded.blobs",
                params![
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
        plugin: String,
        key: String,
        revision: u64,
    ) -> Result<Written, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let before: Option<i64> = transaction
                .query_row(
                    "SELECT revision FROM plugin_values
                     WHERE plugin = ?1 AND key = ?2",
                    params![plugin, key],
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
                "DELETE FROM plugin_values WHERE plugin = ?1 AND key = ?2",
                params![plugin, key],
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
    ) -> Result<BTreeMap<String, Vec<HostDirectory>>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT plugin, name, files, workspace_id FROM plugin_directories ORDER BY plugin, name",
            )?;
            let mut rows = statement.query([])?;
            let mut sets: BTreeMap<String, Vec<HostDirectory>> = BTreeMap::new();
            while let Some(row) = rows.next()? {
                let files: String = row.get(2)?;
                let workspace: Option<String> = row.get(3)?;
                let directory = HostDirectory {
                    name: row.get(1)?,
                    files: decode(DIRECTORIES, "files", serde_json::from_str(&files))?,
                    workspace: workspace
                        .map(|workspace| decode(DIRECTORIES, "workspace_id", WorkspaceId::try_from(workspace)))
                        .transpose()?,
                };
                sets.entry(row.get(0)?).or_default().push(directory);
            }
            Ok(sets)
        })
        .await
    }

    /// Replaces `plugin`'s Host directories for `user` whole, in one
    /// transaction. A directory of a workspace that no longer exists is
    /// dropped, as the workspace's deletion drops those it had.
    pub async fn set_plugin_directories(
        &self,
        plugin: String,
        directories: Vec<HostDirectory>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM plugin_directories WHERE plugin = ?1",
                params![plugin],
            )?;
            for directory in &directories {
                let files = serde_json::to_string(&directory.files)
                    .expect("a directory's files encode as JSON");
                transaction.execute(
                    "INSERT INTO plugin_directories (plugin, name, digest, files, workspace_id)
                     SELECT ?1, ?2, ?3, ?4, ?5
                     WHERE ?5 IS NULL OR EXISTS (SELECT 1 FROM workspaces WHERE id = ?5)",
                    params![
                        plugin,
                        directory.name,
                        directory.digest(),
                        files,
                        directory.workspace.as_ref().map(WorkspaceId::as_str)
                    ],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }
}

/// The blobs `user`'s plugins name: each value's list, and the files of each
/// Host directory.
pub(crate) fn plugin_blobs(
    connection: &Connection,
) -> Result<Vec<BlobRef>, StorageError> {
    let mut blobs = Vec::new();
    let mut values = connection.prepare_cached("SELECT blobs FROM plugin_values")?;
    let mut rows = values.query([])?;
    while let Some(row) = rows.next()? {
        let named: Vec<BlobRef> =
            decode(TABLE, "blobs", serde_json::from_str(&row.get::<_, String>(0)?))?;
        blobs.extend(named);
    }
    let mut directories =
        connection.prepare_cached("SELECT files FROM plugin_directories")?;
    let mut rows = directories.query([])?;
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
