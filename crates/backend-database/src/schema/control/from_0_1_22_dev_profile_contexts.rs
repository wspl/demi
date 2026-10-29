//! From the control schema whose API-key entries sealed their bare configuration,
//! format 1, to provider configuration format 2, which tells a compatible entry
//! by its naming no vendor
//! (`crates/backend-user-shard/src/provider_settings.rs`).
//!
//! The vault opens, decodes and checks each one and seals it again. One it cannot
//! read stops the migration, so the database stays as it was.

use rusqlite::Transaction;

use crate::providers::ResealSealed;
use demi_web_api_protocol::ids::ProviderId;
use super::super::{Migration, corrupt, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Reseal(control_sealed_config_format_2);

fn control_sealed_config_format_2(
    transaction: &Transaction<'_>,
    seals: &dyn ResealSealed,
) -> rusqlite::Result<()> {
    let rows = transaction
        .prepare("SELECT id, provider_type, config FROM providers WHERE config IS NOT NULL")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE providers SET config = ?2 WHERE id = ?1")?;
    for (id, family, sealed) in rows {
        let entry = ProviderId::try_from(id.clone()).map_err(corrupt)?;
        let resealed = seals
            .provider_config(&entry, &family, &sealed)
            .map_err(|reason| corrupt_text(&format!("the configuration of entry {id}: {reason}")))?;
        update.execute(rusqlite::params![id, resealed])?;
    }
    Ok(())
}
