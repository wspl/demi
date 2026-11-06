//! Which user's core holds what a request names before anyone signed in: a
//! device a runner's token names, and a schedule its URL names.
//!
//! The core holds the device and the schedule; these rows only route. A paired
//! device's and a schedule's row is written with its record, a Cloud's when it
//! first boots, and a row whose record is gone finds nothing in the core.

use demi_shared_types::ScheduleId;
use demi_web_api_protocol::ids::{DeviceId, UserId};
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::decode;
use super::control::ControlService;

impl ControlService {
    /// Records that `user`'s core holds `device`, with the hash of its token;
    /// a Cloud has none until it boots.
    pub async fn record_device_owner(
        &self,
        device: DeviceId,
        user: UserId,
        token: Option<TokenHash>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO device_owners (device_id, user_id, token_hash) VALUES (?1, ?2, ?3)
                 ON CONFLICT (device_id) DO UPDATE SET token_hash = coalesce(excluded.token_hash, token_hash)",
                params![device.as_str(), user.as_str(), token.as_ref().map(TokenHash::as_str)],
            )?;
            Ok(())
        })
        .await
    }

    /// Whose core holds the device whose current token has this hash, and
    /// which device it is.
    pub async fn device_by_token(&self, token: TokenHash) -> Result<Option<(DeviceId, UserId)>, StorageError> {
        self.call(move |connection, _| {
            let found: Option<(String, String)> = connection
                .query_row(
                    "SELECT device_id, user_id FROM device_owners WHERE token_hash = ?1",
                    [token.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            found
                .map(|(device, user)| {
                    Ok((
                        decode("device_owners", "device_id", DeviceId::try_from(device))?,
                        decode("device_owners", "user_id", UserId::try_from(user))?,
                    ))
                })
                .transpose()
        })
        .await
    }

    /// Whose core holds `device`.
    pub async fn device_owner(&self, device: DeviceId) -> Result<Option<UserId>, StorageError> {
        self.call(move |connection, _| {
            let user: Option<String> = connection
                .query_row(
                    "SELECT user_id FROM device_owners WHERE device_id = ?1",
                    [device.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            user.map(|user| decode("device_owners", "user_id", UserId::try_from(user))).transpose()
        })
        .await
    }

    /// The users whose cores hold a device.
    pub async fn device_holders(&self) -> Result<Vec<UserId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached("SELECT DISTINCT user_id FROM device_owners")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|user| decode("device_owners", "user_id", UserId::try_from(user?))).collect()
        })
        .await
    }

    /// Replaces the token of `user`'s Cloud device with the one a boot minted:
    /// the token of an earlier boot opens no connection any more.
    pub async fn rotate_device_token(
        &self,
        device: DeviceId,
        user: UserId,
        token: TokenHash,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO device_owners (device_id, user_id, token_hash) VALUES (?1, ?2, ?3)
                 ON CONFLICT (device_id) DO UPDATE SET token_hash = excluded.token_hash",
                params![device.as_str(), user.as_str(), token.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Forgets `device`, whose record its core removed.
    pub async fn forget_device_owner(&self, device: DeviceId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM device_owners WHERE device_id = ?1", [device.as_str()])?;
            Ok(())
        })
        .await
    }

    /// Records that `user`'s core holds the schedule `id`.
    pub async fn record_schedule_owner(&self, id: ScheduleId, user: UserId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO schedule_owners (schedule_id, user_id) VALUES (?1, ?2)
                 ON CONFLICT (schedule_id) DO NOTHING",
                params![id.as_str(), user.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Whose core holds the schedule `id`, for its URL's calls, which carry no
    /// session.
    pub async fn schedule_owner(&self, id: ScheduleId) -> Result<Option<UserId>, StorageError> {
        self.call(move |connection, _| {
            let user: Option<String> = connection
                .query_row(
                    "SELECT user_id FROM schedule_owners WHERE schedule_id = ?1",
                    [id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            user.map(|user| decode("schedule_owners", "user_id", UserId::try_from(user))).transpose()
        })
        .await
    }

    /// Forgets the schedule `id`, whose record its core no longer holds.
    pub async fn forget_schedule_owner(&self, id: ScheduleId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM schedule_owners WHERE schedule_id = ?1", [id.as_str()])?;
            Ok(())
        })
        .await
    }
}
