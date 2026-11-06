//! From the control schema of reply blocks' shares, which no release
//! shipped: a compatible entry makes no images and no
//! videos. The vault opens each API-key entry's configuration of format 2 and
//! seals it again in format 3, without its image and video shapes and a
//! compatible entry's image and video models; one it cannot read stops the
//! migration, so the database stays as it was.

use rusqlite::Transaction;

use crate::schema::ControlSteps;
use demi_web_api_protocol::ids::ProviderId;
use super::super::{Migration, corrupt, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_5979_reply_block_shares);

fn control_from_0_1_5979_reply_block_shares(transaction: &Transaction<'_>, seals: &dyn ControlSteps) -> rusqlite::Result<()> {
    let mut read = transaction.prepare("SELECT id, config FROM providers WHERE credential_kind = 'api_key' AND config IS NOT NULL")?;
    let mut update = transaction.prepare("UPDATE providers SET config = ?2 WHERE id = ?1")?;
    let mut rows = read.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let sealed: Vec<u8> = row.get(1)?;
        let entry = ProviderId::try_from(id.clone()).map_err(corrupt)?;
        let resealed = seals
            .provider_config_format_3(&entry, &sealed)
            .map_err(|reason| corrupt_text(&format!("the configuration of entry {id}: {reason}")))?;
        if let Some(resealed) = resealed {
            update.execute(rusqlite::params![id, resealed])?;
        }
    }
    Ok(())
}
