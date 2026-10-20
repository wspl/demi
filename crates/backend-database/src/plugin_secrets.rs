//! The plugins' secrets and OAuth clients
//! (`crates/backend-plugins/src/secrets.rs`;
//! `crates/backend/src/config/secret.rs`).
//!
//! The module stores:
//!
//! - each plugin's sealed values for a user, by name, each with the revision a
//!   conditional write names;
//! - the public client each plugin registered with its vendor's authorization
//!   server, with the redirect URI it was registered with.
//!
//! The control database stores a secret only sealed; the plugin host seals and
//! opens it.

use demi_web_api_protocol::ids::UserId;
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;
use super::plugin_values::Written;

const TABLE: &str = "plugin_secrets";

/// A sealed secret and its revision.
#[derive(Debug, Clone, PartialEq)]
pub struct SealedSecret {
    pub sealed: Vec<u8>,
    pub revision: u64,
}

/// Where a secret is: its plugin, its user and its name, which its sealing
/// binds it to.
#[derive(Debug, Clone)]
pub struct SecretRow {
    pub user: UserId,
    pub plugin: String,
    pub name: String,
}

/// A client a plugin registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub redirect_uri: String,
    pub client_id: String,
}

impl ControlService {
    /// The secret at `row`.
    pub async fn plugin_secret(&self, row: SecretRow) -> Result<Option<SealedSecret>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT sealed, revision FROM plugin_secrets
                 WHERE user_id = ?1 AND plugin = ?2 AND name = ?3",
            )?;
            let found = statement
                .query_row(params![row.user.as_str(), row.plugin, row.name], |found| {
                    Ok((found.get::<_, Vec<u8>>(0)?, found.get::<_, i64>(1)?))
                })
                .optional()?;
            found
                .map(|(sealed, revision)| {
                    Ok(SealedSecret {
                        sealed,
                        revision: decode(TABLE, "revision", u64::try_from(revision))?,
                    })
                })
                .transpose()
        })
        .await
    }

    /// Writes the secret at `row` if it is still at `revision`, none for a
    /// secret that does not exist yet, in one transaction, so two writes
    /// never build on the same revision.
    pub async fn write_plugin_secret(
        &self,
        row: SecretRow,
        sealed: Vec<u8>,
        revision: Option<u64>,
    ) -> Result<Written, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if stored_revision(&transaction, &row)? != revision {
                return Ok(Written::Conflict);
            }
            let next = revision.map_or(1, |revision| revision + 1);
            transaction.execute(
                "INSERT INTO plugin_secrets (user_id, plugin, name, sealed, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (user_id, plugin, name) DO UPDATE SET
                   sealed = excluded.sealed,
                   revision = excluded.revision",
                params![
                    row.user.as_str(),
                    row.plugin,
                    row.name,
                    sealed,
                    i64::try_from(next).unwrap_or(i64::MAX),
                ],
            )?;
            transaction.commit()?;
            Ok(Written::Revision(next))
        })
        .await
    }

    /// Removes the secret at `row` if it is still at `revision`, in one
    /// transaction. Answers [`Written::Revision`] with the removed revision.
    pub async fn remove_plugin_secret(
        &self,
        row: SecretRow,
        revision: u64,
    ) -> Result<Written, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if stored_revision(&transaction, &row)? != Some(revision) {
                return Ok(Written::Conflict);
            }
            transaction.execute(
                "DELETE FROM plugin_secrets WHERE user_id = ?1 AND plugin = ?2 AND name = ?3",
                params![row.user.as_str(), row.plugin, row.name],
            )?;
            transaction.commit()?;
            Ok(Written::Revision(revision))
        })
        .await
    }

    /// The client `plugin` registered with `issuer`.
    pub async fn plugin_oauth_client(
        &self,
        plugin: String,
        issuer: String,
    ) -> Result<Option<OAuthClient>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT redirect_uri, client_id FROM plugin_oauth_clients
                 WHERE plugin = ?1 AND issuer = ?2",
            )?;
            Ok(statement
                .query_row(params![plugin, issuer], |row| {
                    Ok(OAuthClient {
                        redirect_uri: row.get(0)?,
                        client_id: row.get(1)?,
                    })
                })
                .optional()?)
        })
        .await
    }

    /// Records the client `plugin` registered with `issuer`, in place of
    /// one it registered before.
    pub async fn set_plugin_oauth_client(
        &self,
        plugin: String,
        issuer: String,
        client: OAuthClient,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO plugin_oauth_clients (plugin, issuer, redirect_uri, client_id)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (plugin, issuer) DO UPDATE SET
                   redirect_uri = excluded.redirect_uri,
                   client_id = excluded.client_id",
                params![plugin, issuer, client.redirect_uri, client.client_id],
            )?;
            Ok(())
        })
        .await
    }

    /// Forgets the client `client_id` of `plugin` at `issuer`, which the
    /// vendor no longer knows; a client registered since stays.
    pub async fn forget_plugin_oauth_client(
        &self,
        plugin: String,
        issuer: String,
        client_id: String,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM plugin_oauth_clients
                 WHERE plugin = ?1 AND issuer = ?2 AND client_id = ?3",
                params![plugin, issuer, client_id],
            )?;
            Ok(())
        })
        .await
    }
}

fn stored_revision(
    transaction: &rusqlite::Transaction<'_>,
    row: &SecretRow,
) -> Result<Option<u64>, StorageError> {
    let stored: Option<i64> = transaction
        .query_row(
            "SELECT revision FROM plugin_secrets WHERE user_id = ?1 AND plugin = ?2 AND name = ?3",
            params![row.user.as_str(), row.plugin, row.name],
            |found| found.get(0),
        )
        .optional()?;
    stored
        .map(|revision| decode(TABLE, "revision", u64::try_from(revision)))
        .transpose()
}
