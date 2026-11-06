//! The user's credentials in their core.
//!
//! - What Settings › Credentials lists, creates, changes and deletes.
//! - The sealing of their values by the product's one owner of sealing.
//! - The expiry of the ones kept for a while: one timer for the user's nearest
//!   expiry, which each change of the credentials rearms. A removal past due,
//!   such as one that came while the backend was down, happens as the timer
//!   starts.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use demi_core_store::StorageError;
use demi_core_store::credentials::{
    CredentialChange, CredentialRefusal, CredentialSeal, NewCredential as NewRecord, Saved,
    keep_until,
};
use demi_core_sync::Part;
use demi_core_providers::vault::seal::{Row, VaultKey};
use demi_web_api_protocol::credentials::{
    CredentialDto, CredentialField, CredentialPatch, NewCredential,
};
use demi_web_api_protocol::ids::{UserCredentialId, UserId};
use tokio::sync::Notify;

use super::Core;

/// Seals and opens credentials' values under the instance's key, bound to
/// the user and the credential's name
/// (`crates/backend/src/config/secret.rs`).
pub(crate) struct VaultSeal(pub(crate) VaultKey);

impl CredentialSeal for VaultSeal {
    fn seal(&self, user: &UserId, name: &str, plaintext: &[u8]) -> Vec<u8> {
        self.0.seal(Row::Credential { user, name }, plaintext)
    }

    fn open(&self, user: &UserId, name: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        self.0.open(Row::Credential { user, name }, sealed).ok()
    }
}

/// The expiry's wake-up: a change of the credentials leaves a permit, so a
/// change made while the timer reads is not missed.
#[derive(Default)]
pub(crate) struct CredentialExpiry(Notify);

/// Why a change of a credential was refused.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error(transparent)]
    Refused(#[from] CredentialRefusal),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl Core {
    /// What seals and opens the values of the user's credentials.
    pub(crate) fn vault_seal(&self) -> Arc<dyn CredentialSeal> {
        Arc::new(VaultSeal(self.services().vault.key().clone()))
    }

    /// The user's credentials changed: every page receives them, and the
    /// expiry looks at their times again.
    pub(crate) fn mark_credentials(&self) {
        self.services().sync.mark(self.user(), Part::Credentials);
        self.credential_expiry.0.notify_one();
    }

    /// The user's credentials as a page lists them, in name order.
    pub async fn credential_list(&self) -> Result<Vec<CredentialDto>, StorageError> {
        self.store().user_credential_list(self.vault_seal())
            .await
    }

    /// `POST /credentials/:id/fields/:field/reveal`: the value of the
    /// credential's field `field`, which its owner copies from a card.
    pub async fn reveal_credential(&self, id: UserCredentialId, field: String) -> Result<String, CredentialError> {
        let (_, mut values) = self
            .store().user_credential_values(id, self.vault_seal())
            .await?
            .ok_or(CredentialRefusal::NotFound)?;
        Ok(values.remove(&field).ok_or(CredentialRefusal::NotFound)?)
    }

    /// `POST /credentials`.
    pub async fn create_credential(&self, new: NewCredential) -> Result<CredentialDto, CredentialError> {
        let store = self.store();
        let mut fields = Vec::with_capacity(new.fields.len());
        let mut values = std::collections::BTreeMap::new();
        for field in new.fields {
            // An empty value is one not typed yet: the field is missing.
            if !field.value.is_empty() {
                values.insert(field.name.clone(), field.value);
            }
            fields.push(CredentialField {
                name: field.name,
                label: field.label,
                kind: field.kind,
            });
        }
        let record = NewRecord {
            name: new.name.into_string(),
            note: new.note.unwrap_or_default(),
            fields,
            values,
            keep_until: keep_until(store.now(), new.keep)?,
        };
        let saved = store.create_user_credential(record, self.vault_seal())
            .await??;
        Ok(self.saved(saved))
    }

    /// `PATCH /credentials/:id`.
    pub async fn change_credential(
        &self,
        id: UserCredentialId,
        patch: CredentialPatch,
    ) -> Result<CredentialDto, CredentialError> {
        let store = self.store();
        let change = CredentialChange {
            name: patch.name.map(|name| name.into_string()),
            note: patch.note,
            fields: patch.fields.map(|fields| fields.0),
            values: patch.values,
            keep_until: patch.keep.map(|keep| keep_until(store.now(), keep)).transpose()?,
        };
        let saved = store.change_user_credential(id, change, self.vault_seal())
            .await??;
        Ok(self.saved(saved))
    }

    /// A credential was saved: every page receives it, and each
    /// conversation with a credential request of its name waiting reads its
    /// requests again, whose renewal follows it.
    fn saved(&self, saved: Saved) -> CredentialDto {
        for conversation in &saved.conversations {
            demi_core_permissions::changed(self.permission_core(), conversation);
        }
        self.mark_credentials();
        saved.credential
    }

    /// `DELETE /credentials/:id`, and `demi credentials remove`: the
    /// credential goes, and a waiting request of its name no longer renews it.
    pub async fn remove_credential(&self, id: UserCredentialId) -> Result<(), CredentialError> {
        let removed = self
            .store().remove_user_credential(id)
            .await?
            .ok_or(CredentialRefusal::NotFound)?;
        for conversation in &removed {
            demi_core_permissions::changed(self.permission_core(), conversation);
        }
        self.mark_credentials();
        Ok(())
    }

    /// Removes the user's credentials as their times come, until the core
    /// closes.
    ///
    /// It removes what is past due, sleeps until the nearest time left, and looks
    /// again when that time comes or a change of the credentials rearms it. A
    /// removal that fails waits for the next change or the next start, which
    /// removes what is past due.
    pub(crate) async fn follow_credential_expiry(self: Rc<Self>) {
        loop {
            let store = self.store();
            let next = match store.expire_user_credentials().await {
                Ok(expired) => {
                    if expired.removed {
                        self.services().sync.mark(self.user(), Part::Credentials);
                        for conversation in &expired.conversations {
                            demi_core_permissions::changed(self.permission_core(), conversation);
                        }
                    }
                    expired.next
                }
                Err(error) => {
                    tracing::warn!(event = "credentials.expiry_failed", user = %self.user(), %error, "the expired credentials were not removed");
                    None
                }
            };
            let wait = next.map(|next| {
                let left = next.to_jiff().duration_since(store.now().to_jiff());
                // A time already past is due at once.
                Duration::try_from(left).unwrap_or(Duration::ZERO)
            });
            tokio::select! {
                () = async {
                    match wait {
                        Some(wait) => tokio::time::sleep(wait).await,
                        None => std::future::pending().await,
                    }
                } => {}
                () = self.credential_expiry.0.notified() => {}
                () = self.closed() => return,
            }
        }
    }
}
