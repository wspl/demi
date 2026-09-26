//! The expose records in the user's shard (`expose.md` § The expose record,
//! § Lifetime; `web-api.md` § Exposes): creation on a connected device, the
//! owner's list, renewal and removal, which the routes and the commands
//! share, and the destruction that ends an expose's connections. Every
//! surface shows an expose with its URL, whose scheme and port are the
//! backend's public URL's. Without an expose domain the instance has no
//! exposes: creation is refused as unavailable, and there is none to list,
//! renew or remove.

use demi_web_api::devices::DeviceKind;
use demi_web_api::error::ErrorCode;
use demi_web_api::exposes::{ExposeAddress, ExposeDto};
use demi_web_api::ids::{DeviceId, ExposeId};
use jiff::SignedDuration;
use url::Url;

use super::ExposeDomain;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::exposes::ExposeRecord;

/// How long an expose lives from its creation or its last renewal.
pub(super) const LIFETIME: SignedDuration = SignedDuration::from_hours(1);

/// Why an expose operation was refused.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ExposeError {
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

impl ExposeError {
    /// The code and HTTP status a refusal answers with (`web-api.md`
    /// § Exposes); none for a failure the caller could not cause.
    pub(crate) fn code(&self) -> Option<(ErrorCode, u16)> {
        match self {
            Self::Unavailable => Some((ErrorCode::ExposeUnavailable, 409)),
            Self::DeviceNotFound => Some((ErrorCode::DeviceNotFound, 404)),
            Self::DeviceOffline(_) => Some((ErrorCode::DeviceOffline, 409)),
            Self::NotFound(_) => Some((ErrorCode::ExposeNotFound, 404)),
            Self::Storage(_) => None,
        }
    }
}

/// A new expose's id: 128 random bits as 26 lowercase base32 characters.
fn new_expose_id() -> ExposeId {
    let bits: [u8; 16] = rand::random();
    let text = data_encoding::BASE32_NOPAD.encode(&bits).to_ascii_lowercase();
    ExposeId::try_from(text).expect("128 bits are 26 base32 characters")
}

/// The public URL of the expose `id`: its hostname under `domain`, with the
/// scheme of `backend` and its port unless that is the scheme's default.
pub(crate) fn expose_url(id: &ExposeId, domain: &ExposeDomain, backend: &Url) -> String {
    let port = backend.port().map(|port| format!(":{port}")).unwrap_or_default();
    format!("{}://{id}.{}{port}/", backend.scheme(), domain.as_str())
}

impl Shard {
    /// A new expose of `address` on the user's device `device`, for an
    /// hour: the device is connected, and a Cloud is running.
    pub(crate) async fn add_expose(&self, device: &DeviceId, address: ExposeAddress) -> Result<ExposeDto, ExposeError> {
        let services = self.services();
        let record = services
            .control
            .device(device.clone())
            .await?
            .filter(|record| record.user == *self.user())
            .ok_or(ExposeError::DeviceNotFound)?;
        let domain = services.expose_domain.as_ref().ok_or(ExposeError::Unavailable)?;
        let connected = match record.kind {
            DeviceKind::User => self.devices().online(device),
            DeviceKind::Managed => self.devices().online(device) && self.cloud().runs(device),
        };
        if !connected {
            return Err(ExposeError::DeviceOffline(device.clone()));
        }
        // The database takes the record in the step that checked the
        // device: a Cloud that stops after the check destroys it with the
        // Cloud's other exposes.
        let created = services
            .control
            .create_expose(new_expose_id(), self.user().clone(), device.clone(), address, LIFETIME)
            .await?;
        Ok(self.expose_dto(created, domain))
    }

    /// The user's live exposes, soonest expiry first; the expired ones are
    /// destroyed.
    pub(crate) async fn list_exposes(&self) -> Result<Vec<ExposeDto>, StorageError> {
        let Some(domain) = &self.services().expose_domain else {
            return Ok(Vec::new());
        };
        let listed = self.services().control.user_exposes(self.user().clone()).await?;
        self.exposes().end(&listed.expired);
        Ok(listed
            .live
            .into_iter()
            .map(|record| self.expose_dto(record, domain))
            .collect())
    }

    /// Moves the expiry of the user's expose `id` to an hour from now.
    pub(crate) async fn renew_expose(&self, id: &ExposeId) -> Result<ExposeDto, ExposeError> {
        let not_found = || ExposeError::NotFound(id.to_string());
        let domain = self.services().expose_domain.as_ref().ok_or_else(not_found)?;
        let record = self.owned_expose(id).await?.ok_or_else(not_found)?;
        if self.destroy_if_expired(&record).await? {
            return Err(not_found());
        }
        let renewed = self
            .services()
            .control
            .renew_expose(id.clone(), self.user().clone(), LIFETIME)
            .await?
            .ok_or_else(not_found)?;
        Ok(self.expose_dto(renewed, domain))
    }

    /// Destroys the user's expose `id` at once. One that had expired is
    /// destroyed too, and answers as not found.
    pub(crate) async fn remove_expose(&self, id: &ExposeId) -> Result<(), ExposeError> {
        let not_found = || ExposeError::NotFound(id.to_string());
        self.services().expose_domain.as_ref().ok_or_else(not_found)?;
        let record = self.owned_expose(id).await?.ok_or_else(not_found)?;
        if self.destroy_if_expired(&record).await? {
            return Err(not_found());
        }
        self.destroy_expose(id).await?;
        Ok(())
    }

    /// The user's expose `id`, expired or not.
    pub(super) async fn owned_expose(&self, id: &ExposeId) -> Result<Option<ExposeRecord>, StorageError> {
        let record = self.services().control.expose(id.clone()).await?;
        Ok(record.filter(|record| record.user == *self.user()))
    }

    /// Destroys the user's `record` if it expired (`expose.md` § Lifetime),
    /// as every read of one does; answers whether it had.
    pub(super) async fn destroy_if_expired(&self, record: &ExposeRecord) -> Result<bool, StorageError> {
        if self.services().clock.now() < record.expires_at {
            return Ok(false);
        }
        self.destroy_expose(&record.id).await?;
        Ok(true)
    }

    /// Destroys the user's expose `id`: its record goes, and its
    /// connections end.
    async fn destroy_expose(&self, id: &ExposeId) -> Result<(), StorageError> {
        self.services().control.delete_expose(id.clone()).await?;
        self.exposes().end([id]);
        Ok(())
    }

    /// Destroys every expose on the user's `device`, as the device's
    /// revocation and a Cloud's stop do. A failure is logged: the device's
    /// runner, which goes next, ends the connections all the same, and the
    /// records expire within the hour.
    pub(crate) async fn destroy_exposes_on(&self, device: &DeviceId) {
        match self.services().control.delete_device_exposes(device.clone()).await {
            Ok(ids) => self.exposes().end(&ids),
            Err(error) => tracing::error!(device = %device, "the exposes of a device could not be destroyed: {error}"),
        }
    }

    /// The expose as every surface shows it.
    fn expose_dto(&self, record: ExposeRecord, domain: &ExposeDomain) -> ExposeDto {
        let backend = self
            .services()
            .public_url
            .get()
            .expect("the backend listens before it serves a request or a command");
        ExposeDto {
            url: expose_url(&record.id, domain, backend.url()),
            id: record.id,
            device_id: record.device,
            address: record.address,
            created_at: record.created_at,
            expires_at: record.expires_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expose_id_is_a_dns_label_of_128_random_bits() {
        let id = new_expose_id();
        assert_eq!(id.as_str().len(), 26);
        assert_ne!(id, new_expose_id());
    }

    #[test]
    fn an_expose_url_takes_the_scheme_and_any_other_than_the_default_port_from_the_backend_url() {
        let id = ExposeId::try_from("k7x2maqw4p3s6tavaw2y4z6aab").unwrap();
        let local: ExposeDomain = "expose.localhost".parse().unwrap();
        let backend = Url::parse("http://localhost:3271").unwrap();
        assert_eq!(expose_url(&id, &local, &backend), format!("http://{id}.expose.localhost:3271/"));
        let public: ExposeDomain = "expose.demi.example".parse().unwrap();
        for backend in ["https://demi.example", "https://demi.example:443/", "https://demi.example/api"] {
            let backend = Url::parse(backend).unwrap();
            assert_eq!(expose_url(&id, &public, &backend), format!("https://{id}.expose.demi.example/"));
        }
    }
}
