//! The expose records in the user's shard (`expose.md` § The expose record,
//! § Lifetime): creation on a connected device, the owner's list, renewal
//! and removal, which the `expose` plugin makes through its port, and the
//! destruction that ends an expose's connections. Every expose is shown
//! with its URL, whose scheme and port are the backend's public URL's.
//! Without an expose domain the instance has no exposes: creation is
//! refused as unavailable, and there is none to list, renew or remove. Each
//! change is reported to the shard, whose plugins' page states follow it.

use demi_backend_database::StorageError;
use demi_backend_database::exposes::ExposeRecord;
use demi_web_api_protocol::exposes::ExposeAddress;
use demi_web_api_protocol::ids::{DeviceId, ExposeId};
use jiff::SignedDuration;
use url::Url;

use crate::ExposeShard;
use crate::domain::ExposeDomain;

/// A live expose and its public URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expose {
    pub record: ExposeRecord,
    pub url: String,
}

/// Why an expose operation was refused.
#[derive(Debug, thiserror::Error)]
pub enum ExposeError {
    /// The instance has no expose domain.
    #[error("This backend has no expose domain configured (DEMI_EXPOSE_DOMAIN)")]
    Unavailable,
    /// The device is not the caller's.
    #[error("No such device")]
    DeviceNotFound,
    /// The device's runner is not connected, or the Cloud is not running.
    #[error("The device {0} is offline; connect it before exposing a service")]
    DeviceOffline(DeviceId),
    /// The caller has no expose of this id, or it expired.
    #[error("No expose {0}")]
    NotFound(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// A new expose's id: 128 random bits as 26 lowercase base32 characters.
fn new_expose_id() -> ExposeId {
    let bits: [u8; 16] = rand::random();
    let text = data_encoding::BASE32_NOPAD
        .encode(&bits)
        .to_ascii_lowercase();
    ExposeId::try_from(text).expect("128 bits are 26 base32 characters")
}

/// The public URL of the expose `id`: its hostname under `domain`, with the
/// scheme of `backend` and its port unless that is the scheme's default.
pub fn expose_url(id: &ExposeId, domain: &ExposeDomain, backend: &Url) -> String {
    let port = backend
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{}://{id}.{}{port}/", backend.scheme(), domain.as_str())
}

impl dyn ExposeShard {
    /// A new expose of `address` on the user's device `device`, for
    /// `lifetime`: the device is connected, and a Cloud is running.
    pub async fn add_expose(
        &self,
        device: &DeviceId,
        address: ExposeAddress,
        lifetime: SignedDuration,
    ) -> Result<Expose, ExposeError> {
        let record = self
            .control()
            .device(device.clone())
            .await?
            .filter(|record| record.user == *self.user())
            .ok_or(ExposeError::DeviceNotFound)?;
        let domain = self.domain().ok_or(ExposeError::Unavailable)?;
        if !self.device_connected(&record) {
            return Err(ExposeError::DeviceOffline(device.clone()));
        }
        // The database takes the record in the step that checked the
        // device: a Cloud that stops after the check destroys it with the
        // Cloud's other exposes.
        let created = self
            .control()
            .create_expose(
                new_expose_id(),
                self.user().clone(),
                device.clone(),
                address,
                lifetime,
            )
            .await?;
        self.exposes_changed();
        Ok(self.expose(created, domain))
    }

    /// The user's live exposes, soonest expiry first; the expired ones are
    /// destroyed.
    pub async fn list_exposes(&self) -> Result<Vec<Expose>, StorageError> {
        let Some(domain) = self.domain() else {
            return Ok(Vec::new());
        };
        let listed = self.control().user_exposes(self.user().clone()).await?;
        self.exposes().end(&listed.expired);
        Ok(listed
            .live
            .into_iter()
            .map(|record| self.expose(record, domain))
            .collect())
    }

    /// Moves the expiry of the user's expose `id` to `lifetime` from now.
    pub async fn renew_expose(
        &self,
        id: &ExposeId,
        lifetime: SignedDuration,
    ) -> Result<Expose, ExposeError> {
        let not_found = || ExposeError::NotFound(id.to_string());
        let domain = self.domain().ok_or_else(not_found)?;
        let record = self.owned_expose(id).await?.ok_or_else(not_found)?;
        if self.destroy_if_expired(&record).await? {
            return Err(not_found());
        }
        let renewed = self
            .control()
            .renew_expose(id.clone(), self.user().clone(), lifetime)
            .await?
            .ok_or_else(not_found)?;
        self.exposes_changed();
        Ok(self.expose(renewed, domain))
    }

    /// Destroys the user's expose `id` at once. One that had expired is
    /// destroyed too, and answers as not found.
    pub async fn remove_expose(&self, id: &ExposeId) -> Result<(), ExposeError> {
        let not_found = || ExposeError::NotFound(id.to_string());
        self.domain().ok_or_else(not_found)?;
        let record = self.owned_expose(id).await?.ok_or_else(not_found)?;
        if self.destroy_if_expired(&record).await? {
            return Err(not_found());
        }
        self.destroy_expose(id).await?;
        Ok(())
    }

    /// The user's expose `id`, expired or not.
    pub(crate) async fn owned_expose(
        &self,
        id: &ExposeId,
    ) -> Result<Option<ExposeRecord>, StorageError> {
        let record = self.control().expose(id.clone()).await?;
        Ok(record.filter(|record| record.user == *self.user()))
    }

    /// Destroys the user's `record` if it expired (`expose.md` § Lifetime),
    /// as every read of one does; answers whether it had.
    pub(crate) async fn destroy_if_expired(
        &self,
        record: &ExposeRecord,
    ) -> Result<bool, StorageError> {
        if self.clock().now() < record.expires_at {
            return Ok(false);
        }
        self.destroy_expose(&record.id).await?;
        Ok(true)
    }

    /// Destroys the user's expose `id`: its record goes, and its
    /// connections end.
    async fn destroy_expose(&self, id: &ExposeId) -> Result<(), StorageError> {
        self.control().delete_expose(id.clone()).await?;
        self.exposes_changed();
        self.exposes().end([id]);
        Ok(())
    }

    /// Destroys every expose on the user's `device`, as the device's
    /// revocation and a Cloud's stop do. A failure is logged: the device's
    /// runner, which goes next, ends the connections all the same, and the
    /// records expire within the hour.
    pub async fn destroy_exposes_on(&self, device: &DeviceId) {
        match self.control().delete_device_exposes(device.clone()).await {
            Ok(ids) => {
                if !ids.is_empty() {
                    self.exposes_changed();
                }
                self.exposes().end(&ids);
            }
            Err(error) => {
                tracing::error!(device = %device, "the exposes of a device could not be destroyed: {error}")
            }
        }
    }

    /// The expose with its URL.
    fn expose(&self, record: ExposeRecord, domain: &ExposeDomain) -> Expose {
        Expose {
            url: expose_url(&record.id, domain, self.public_url()),
            record,
        }
    }
}
