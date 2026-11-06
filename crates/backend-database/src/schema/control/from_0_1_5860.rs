//! From 0.1.5860's control schema: a provider entry connects through one of
//! its owner's devices, and the exposed ports go.
//!
//! - An API-key entry names the device its requests go out from, none for
//!   the server. One whose endpoint named one of its owner's devices by its
//!   id, `http://<device id>:<port>`, connects through that device, its
//!   endpoint on the device itself: `http://127.0.0.1:<port>`, or
//!   `https://localhost:<port>`, with the rest of its address. The vault
//!   opens each one to read its endpoint; one it cannot read stops the
//!   migration, so the database stays as it was.
//! - The exposed ports go, and so does the permission to manage them, with
//!   its grants and every request for it still stored.

use std::collections::HashSet;

use rusqlite::Transaction;

use crate::schema::ControlSteps;
use demi_web_api_protocol::ids::ProviderId;
use super::super::{Migration, corrupt, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_5860);

fn control_from_0_1_5860(transaction: &Transaction<'_>, seals: &dyn ControlSteps) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "ALTER TABLE providers ADD COLUMN through_device_id TEXT
           CHECK (through_device_id IS NULL OR credential_kind = 'api_key');",
    )?;
    let devices: HashSet<(String, String)> = transaction
        .prepare("SELECT user_id, id FROM devices")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut read = transaction.prepare("SELECT id, owner_user_id, config FROM providers WHERE credential_kind = 'api_key' AND config IS NOT NULL")?;
    let mut update = transaction.prepare("UPDATE providers SET config = ?2, through_device_id = ?3 WHERE id = ?1")?;
    let mut rows = read.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let owner: String = row.get(1)?;
        let sealed: Vec<u8> = row.get(2)?;
        let entry = ProviderId::try_from(id.clone()).map_err(corrupt)?;
        let owned = |device: &str| devices.contains(&(owner.clone(), device.to_owned()));
        let moved = seals
            .provider_endpoint_on_device(&entry, &sealed, &owned)
            .map_err(|reason| corrupt_text(&format!("the configuration of entry {id}: {reason}")))?;
        if let Some((device, resealed)) = moved {
            update.execute(rusqlite::params![id, resealed, device])?;
        }
    }
    transaction.execute_batch(
        "DROP TABLE exposed_ports;
         DELETE FROM permission_grants WHERE category = 'host.expose';
         DELETE FROM permission_requests
           WHERE kind = 'permission'
             AND EXISTS (SELECT 1 FROM json_each(categories) WHERE value = 'host.expose');",
    )
}
