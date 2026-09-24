//! Expose records (`storage.md` § Control records; `expose.md` § The expose
//! record): a service on one of the user's devices, reachable under its
//! public hostname until it expires.

use demi_core::Timestamp;
use demi_web_api::exposes::ExposeAddress;
use demi_web_api::ids::{DeviceId, ExposeId, UserId};
use rusqlite::{OptionalExtension, Row};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

const EXPOSE_COLUMNS: &str = "id, user_id, device_id, address, created_at, expires_at";

/// An `exposes` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExposeRecord {
    pub(crate) id: ExposeId,
    pub(crate) user: UserId,
    pub(crate) device: DeviceId,
    pub(crate) address: ExposeAddress,
    pub(crate) created_at: Timestamp,
    pub(crate) expires_at: Timestamp,
}

impl ControlService {
    /// The expose `id`, expired or not.
    pub(crate) async fn expose(&self, id: ExposeId) -> Result<Option<ExposeRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement =
                connection.prepare_cached(&format!("SELECT {EXPOSE_COLUMNS} FROM exposes WHERE id = ?1"))?;
            statement
                .query_row([id.as_str()], |row| Ok(expose_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }
}

fn expose_row(row: &Row<'_>) -> Result<ExposeRecord, StorageError> {
    const TABLE: &str = "exposes";
    Ok(ExposeRecord {
        id: decode(TABLE, "id", ExposeId::try_from(row.get::<_, String>("id")?))?,
        user: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        device: decode(TABLE, "device_id", DeviceId::try_from(row.get::<_, String>("device_id")?))?,
        address: decode(TABLE, "address", ExposeAddress::try_from(row.get::<_, String>("address")?))?,
        created_at: instant(row, TABLE, "created_at")?,
        expires_at: instant(row, TABLE, "expires_at")?,
    })
}
