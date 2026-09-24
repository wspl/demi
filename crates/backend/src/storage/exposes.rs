//! Expose records (`storage.md` § Control records; `expose.md` § The expose
//! record): a service on one of the user's devices, reachable under its
//! public hostname until it expires. A record is expired once its expiry is
//! not after the operation's time; expiry, removal, a Cloud stop and a
//! device revocation delete rows, and nothing updates a row except renewal.

use demi_core::Timestamp;
use demi_web_api::exposes::ExposeAddress;
use demi_web_api::ids::{DeviceId, ExposeId, UserId};
use jiff::SignedDuration;
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::{ControlService, later};

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

/// A user's exposes as a listing leaves them: the live ones, soonest expiry
/// first, and the ids of the expired ones, which it deleted.
#[derive(Debug)]
pub(crate) struct UserExposes {
    pub(crate) live: Vec<ExposeRecord>,
    pub(crate) expired: Vec<ExposeId>,
}

impl ControlService {
    /// A new expose `id` of `user` on `device`, from now until `lifetime`
    /// from now.
    pub(crate) async fn create_expose(
        &self,
        id: ExposeId,
        user: UserId,
        device: DeviceId,
        address: ExposeAddress,
        lifetime: SignedDuration,
    ) -> Result<ExposeRecord, StorageError> {
        self.call(move |connection, now| {
            let expires_at = later(now, lifetime)?;
            connection.execute(
                "INSERT INTO exposes (id, user_id, device_id, address, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id.as_str(),
                    user.as_str(),
                    device.as_str(),
                    address.as_str(),
                    now.as_millisecond(),
                    expires_at.as_millisecond()
                ],
            )?;
            Ok(ExposeRecord {
                id,
                user,
                device,
                address,
                created_at: now,
                expires_at,
            })
        })
        .await
    }

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

    /// The exposes of `user`, after deleting the expired ones. A listing
    /// that finds none expired writes nothing.
    pub(crate) async fn user_exposes(&self, user: UserId) -> Result<UserExposes, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let records = {
                let mut statement = transaction.prepare_cached(&format!(
                    "SELECT {EXPOSE_COLUMNS} FROM exposes WHERE user_id = ?1 ORDER BY expires_at, id"
                ))?;
                let mut rows = statement.query([user.as_str()])?;
                let mut records = Vec::new();
                while let Some(row) = rows.next()? {
                    records.push(expose_row(row)?);
                }
                records
            };
            let (expired, live): (Vec<ExposeRecord>, Vec<ExposeRecord>) =
                records.into_iter().partition(|record| record.expires_at <= now);
            for record in &expired {
                transaction.execute("DELETE FROM exposes WHERE id = ?1", [record.id.as_str()])?;
            }
            transaction.commit()?;
            Ok(UserExposes {
                live,
                expired: expired.into_iter().map(|record| record.id).collect(),
            })
        })
        .await
    }

    /// Moves the expiry of the live expose `id` of `user` to `lifetime` from
    /// now; none when `user` has no such live expose.
    pub(crate) async fn renew_expose(
        &self,
        id: ExposeId,
        user: UserId,
        lifetime: SignedDuration,
    ) -> Result<Option<ExposeRecord>, StorageError> {
        self.call(move |connection, now| {
            let expires_at = later(now, lifetime)?;
            let mut statement = connection.prepare_cached(&format!(
                "UPDATE exposes SET expires_at = ?1 WHERE id = ?2 AND user_id = ?3 AND expires_at > ?4
                 RETURNING {EXPOSE_COLUMNS}"
            ))?;
            statement
                .query_row(
                    params![expires_at.as_millisecond(), id.as_str(), user.as_str(), now.as_millisecond()],
                    |row| Ok(expose_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }

    /// Deletes the expose `id`, if there is one.
    pub(crate) async fn delete_expose(&self, id: ExposeId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM exposes WHERE id = ?1", [id.as_str()])?;
            Ok(())
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
