//! Expose records (`storage.md` § Control records; `expose.md` § The expose
//! record): a service on one of the user's devices, reachable under its
//! public hostname until it expires. The numbers the model knows exposes by
//! are the `expose` plugin's values. A record is expired once its expiry is not
//! after the operation's time; expiry, removal, a Cloud stop and a device
//! revocation delete rows, and nothing updates a row except renewal.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::exposes::ExposeAddress;
use demi_web_api_protocol::ids::{DeviceId, ExposeId, UserId};
use jiff::SignedDuration;
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::{ControlService, later};

const EXPOSE_COLUMNS: &str = "id, user_id, device_id, address, created_at, expires_at";

/// An `exposes` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposeRecord {
    pub id: ExposeId,
    pub user: UserId,
    pub device: DeviceId,
    pub address: ExposeAddress,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// A user's exposes as a listing leaves them: the live ones, soonest expiry
/// first, and the ids of the expired ones, which it deleted.
#[derive(Debug)]
pub struct UserExposes {
    pub live: Vec<ExposeRecord>,
    pub expired: Vec<ExposeId>,
}

impl ControlService {
    /// A new expose `id` of `user` on `device`, from now until `lifetime`
    /// from now.
    pub async fn create_expose(
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
    pub async fn expose(&self, id: ExposeId) -> Result<Option<ExposeRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {EXPOSE_COLUMNS} FROM exposes WHERE id = ?1"
            ))?;
            statement
                .query_row([id.as_str()], |row| Ok(expose_row(row)))
                .optional()?
                .transpose()
        })
        .await
    }

    /// The exposes of `user`, after deleting the expired ones. A listing
    /// that finds none expired writes nothing.
    pub async fn user_exposes(&self, user: UserId) -> Result<UserExposes, StorageError> {
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
    pub async fn renew_expose(
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
    pub async fn delete_expose(&self, id: ExposeId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM exposes WHERE id = ?1", [id.as_str()])?;
            Ok(())
        })
        .await
    }

    /// Deletes the expose `id` if it has expired by now, and answers whether
    /// it did. The expiry is checked in the deletion itself, so a renewal that
    /// came after the caller read the record keeps the expose.
    pub async fn delete_expired_expose(&self, id: ExposeId) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let deleted = connection.execute(
                "DELETE FROM exposes WHERE id = ?1 AND expires_at <= ?2",
                params![id.as_str(), now.as_millisecond()],
            )?;
            Ok(deleted > 0)
        })
        .await
    }

    /// Deletes every expose on `device`; answers their ids.
    pub async fn delete_device_exposes(
        &self,
        device: DeviceId,
    ) -> Result<Vec<ExposeId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection
                .prepare_cached("DELETE FROM exposes WHERE device_id = ?1 RETURNING id")?;
            let mut rows = statement.query([device.as_str()])?;
            let mut ids = Vec::new();
            while let Some(row) = rows.next()? {
                ids.push(decode(
                    "exposes",
                    "id",
                    ExposeId::try_from(row.get::<_, String>("id")?),
                )?);
            }
            Ok(ids)
        })
        .await
    }

    /// Deletes the exposes of every user's Cloud, as a backend that starts
    /// does: the machine manager has stopped every Cloud by then.
    pub async fn delete_cloud_exposes(&self) -> Result<(), StorageError> {
        self.call(|connection, _| {
            connection.execute(
                "DELETE FROM exposes WHERE device_id IN (SELECT id FROM devices WHERE kind = 'managed')",
                [],
            )?;
            Ok(())
        })
        .await
    }
}

fn expose_row(row: &Row<'_>) -> Result<ExposeRecord, StorageError> {
    const TABLE: &str = "exposes";
    Ok(ExposeRecord {
        id: decode(TABLE, "id", ExposeId::try_from(row.get::<_, String>("id")?))?,
        user: decode(
            TABLE,
            "user_id",
            UserId::try_from(row.get::<_, String>("user_id")?),
        )?,
        device: decode(
            TABLE,
            "device_id",
            DeviceId::try_from(row.get::<_, String>("device_id")?),
        )?,
        address: decode(
            TABLE,
            "address",
            ExposeAddress::try_from(row.get::<_, String>("address")?),
        )?,
        created_at: instant(row, TABLE, "created_at")?,
        expires_at: instant(row, TABLE, "expires_at")?,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use demi_provider_common::testing::ManualClock;
    use demi_runner_protocol::wire::RunnerPlatform;
    use demi_shared_types::Clock;

    use super::*;
    use crate::accounts::TokenHash;
    use crate::control::testing::master;

    /// An expose renewed after it was read, as a relay or the expiry watch
    /// reads it before deleting it, is not deleted as expired once its first
    /// hour passes; it is once the renewed hour passes (`expose.md` § Lifetime).
    #[tokio::test]
    async fn an_expose_renewed_after_it_was_read_is_not_deleted_as_expired() {
        let data = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(
            Timestamp::from_millisecond(1_790_000_000_000).unwrap(),
        ));
        let control = ControlService::open(&data.path().join("control.sqlite"), clock.clone())
            .await
            .unwrap();
        let user = master(&control).await;
        let device = control
            .create_device(
                user.id.clone(),
                "laptop".into(),
                RunnerPlatform::Linux,
                TokenHash::of("token"),
            )
            .await
            .unwrap();
        let id = ExposeId::try_from("k7x2maqw4p3s6tavaw2y4z6aab").unwrap();
        let hour = SignedDuration::from_hours(1);
        let address = ExposeAddress::try_from("localhost:3000".to_owned()).unwrap();
        control
            .create_expose(id.clone(), user.id.clone(), device.id, address, hour)
            .await
            .unwrap();
        let read = control.expose(id.clone()).await.unwrap().unwrap();
        clock.advance(SignedDuration::from_mins(59));
        control
            .renew_expose(id.clone(), user.id.clone(), hour)
            .await
            .unwrap()
            .unwrap();
        clock.advance(SignedDuration::from_mins(2));
        assert!(
            read.expires_at <= clock.now(),
            "the record read before the renewal looks expired"
        );
        assert!(!control.delete_expired_expose(id.clone()).await.unwrap());
        assert!(control.expose(id.clone()).await.unwrap().is_some());
        clock.advance(hour);
        assert!(control.delete_expired_expose(id.clone()).await.unwrap());
        assert!(control.expose(id).await.unwrap().is_none());
    }
}
