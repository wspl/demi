//! The plugins' secrets (`plugins.md` § Secrets): values beside the plugin's
//! JSON values, with the same versioned compare-and-set, sealed at rest by
//! the product's one owner of sealing with the plugin, the user and the
//! secret's name as the additional data. The plugin reads only the names
//! [`SecretName`](demi_plugin_interface::SecretName) allows; the host keeps
//! a sign-in's tokens under a name outside that rule.

use demi_backend_database::StorageError;
use demi_backend_database::plugin_secrets::SecretRow;
use demi_backend_database::plugin_values::Written;
use demi_plugin_interface::{PortFailure, PortRefusal, StoredSecret};

use crate::user::{Shared, storage};

impl Shared {
    fn secret_row(&self, plugin: usize, name: &str) -> SecretRow {
        SecretRow {
            user: self.user.clone(),
            plugin: self.plugin_id(plugin),
            name: name.to_owned(),
        }
    }

    /// Plugin `plugin`'s secret `name`, opened. A sealed value that does
    /// not open is corrupt, and nothing replaces it.
    pub(crate) async fn read_secret(
        &self,
        plugin: usize,
        name: &str,
    ) -> Result<Option<StoredSecret>, PortFailure> {
        let row = self.secret_row(plugin, name);
        let Some(stored) = self.control.plugin_secret(row).await.map_err(storage)? else {
            return Ok(None);
        };
        let id = self.plugin_id(plugin);
        let opened = self
            .product
            .open(&self.user, &id, name, &stored.sealed)
            .and_then(|plaintext| String::from_utf8(plaintext).ok());
        let Some(value) = opened else {
            return Err(storage(StorageError::Corrupt {
                table: "plugin_secrets",
                column: "sealed",
                reason: format!("the secret \"{name}\" of the plugin \"{id}\" does not open"),
            }));
        };
        Ok(Some(StoredSecret {
            value,
            revision: stored.revision,
        }))
    }

    /// Seals `value` as plugin `plugin`'s secret `name` if it is still at
    /// `revision`, and answers its new revision.
    pub(crate) async fn write_secret(
        &self,
        plugin: usize,
        name: &str,
        value: &str,
        revision: Option<u64>,
    ) -> Result<u64, PortFailure> {
        let row = self.secret_row(plugin, name);
        let sealed = self
            .product
            .seal(&self.user, &row.plugin, name, value.as_bytes());
        match self
            .control
            .write_plugin_secret(row, sealed, revision)
            .await
            .map_err(storage)?
        {
            Written::Revision(revision) => Ok(revision),
            Written::Conflict => Err(PortFailure::Refused(PortRefusal::Conflict)),
        }
    }

    /// Removes plugin `plugin`'s secret `name` if it is still at
    /// `revision`.
    pub(crate) async fn remove_secret(
        &self,
        plugin: usize,
        name: &str,
        revision: u64,
    ) -> Result<(), PortFailure> {
        let row = self.secret_row(plugin, name);
        match self
            .control
            .remove_plugin_secret(row, revision)
            .await
            .map_err(storage)?
        {
            Written::Revision(_) => Ok(()),
            Written::Conflict => Err(PortFailure::Refused(PortRefusal::Conflict)),
        }
    }
}
