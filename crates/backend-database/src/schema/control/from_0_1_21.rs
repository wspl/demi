//! From 0.1.21's control schema ([`CONTROL_FROM_0_1_21`]); and a stored catalog's
//! models no longer say whether they read attachments or video.
//!
//! Their accepted types come from the source's input modalities alone
//! (`crates/shared-types/src/model.rs`), so the two flags go and each model keeps
//! the list it held, null where the source stated none. Each record changed is
//! decoded and checked as the format it becomes before it is written back.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob, corrupt};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_21);

fn control_from_0_1_21(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(CONTROL_FROM_0_1_21)?;
    let records = transaction
        .prepare("SELECT provider_id, record FROM model_catalogs")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE model_catalogs SET record = ?2 WHERE provider_id = ?1")?;
    for (provider, record) in records {
        let mut value: serde_json::Value = serde_json::from_str(&record).map_err(corrupt)?;
        if !without_attachment_flags(&mut value) {
            continue;
        }
        update.execute(rusqlite::params![provider, value.to_string()])?;
    }
    Ok(())
}

/// From 0.1.21's control schema.
///
/// - A conversation keeps the move an agent asked for and an attached host the
///   detach it waits with, none for every conversation 0.1.21 left.
/// - A permission request names its categories, the one category of each request
///   0.1.21 stored. SQLite cannot drop a column an index names, so the requests'
///   table is made anew, as for 0.1.11's conversations.
/// - The web preview is gone, and the deployment's namespace at the preview domain
///   with it.
/// - Users keep personal instructions, which no user migrated from 0.1.21 has.
/// - Yield wakeups are gone, and the index of when each conversation's earliest
///   one is due with them.
/// - Running jobs are recorded by device, none of which 0.1.21 kept, since its
///   jobs ended with their connections.
/// - A device keeps its runner's instance, which no runner of 0.1.21 named.
const CONTROL_FROM_0_1_21: &str = "
DROP INDEX conversations_wakeup;
ALTER TABLE conversations DROP COLUMN wakeup_at;
CREATE TABLE user_instructions (
  user_id TEXT PRIMARY KEY REFERENCES users (id),
  text    TEXT NOT NULL
) STRICT;
ALTER TABLE conversations ADD COLUMN pending_id TEXT;
ALTER TABLE conversations ADD COLUMN pending_kind TEXT CHECK (pending_kind IN ('cloud', 'device', 'workspace'));
ALTER TABLE conversations ADD COLUMN pending_device_id TEXT;
ALTER TABLE conversations ADD COLUMN pending_path TEXT;
ALTER TABLE conversations ADD COLUMN pending_workspace_id TEXT CHECK (
    (pending_kind IS NULL
      AND pending_id IS NULL AND pending_device_id IS NULL AND pending_path IS NULL
      AND pending_workspace_id IS NULL)
    OR (pending_kind = 'cloud' AND pending_id IS NOT NULL
      AND pending_device_id IS NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'device' AND pending_id IS NOT NULL
      AND pending_device_id IS NOT NULL AND pending_path IS NOT NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'workspace' AND pending_id IS NOT NULL
      AND pending_workspace_id IS NOT NULL AND pending_device_id IS NULL AND pending_path IS NULL)
  );
ALTER TABLE conversation_hosts ADD COLUMN detaching INTEGER NOT NULL DEFAULT 0 CHECK (detaching IN (0, 1));

CREATE TABLE permission_requests_next (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  categories        TEXT NOT NULL,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  decision          TEXT CHECK (decision IN ('allowed', 'denied')),
  decided_at        INTEGER,
  CHECK ((agent_number IS NULL) = (agent_description IS NULL)),
  CHECK ((decision IS NULL) = (decided_at IS NULL))
) STRICT;
INSERT INTO permission_requests_next
  (id, conversation_id, categories, command, node_id, agent_number, agent_description, created_at, decision, decided_at)
  SELECT id, conversation_id, json_array(category), command, node_id, agent_number, agent_description, created_at, decision, decided_at
  FROM permission_requests;
DROP TABLE permission_requests;
ALTER TABLE permission_requests_next RENAME TO permission_requests;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);
DROP TABLE preview_namespace;
CREATE TABLE running_jobs (
  job_id          TEXT PRIMARY KEY,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;
CREATE INDEX running_jobs_of_device ON running_jobs (device_id);
ALTER TABLE devices ADD COLUMN runner_instance INTEGER;
ALTER TABLE devices ADD COLUMN cloud_started_at INTEGER;
";

/// Removes every catalog model's `supportsAttachments` and `supportsVideo`
/// from `value`; true when it held some.
fn without_attachment_flags(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => {
            let mut changed = fields.remove("supportsAttachments").is_some();
            changed |= fields.remove("supportsVideo").is_some();
            for field in fields.values_mut() {
                changed |= without_attachment_flags(field);
            }
            changed
        }
        serde_json::Value::Array(items) => items
            .iter_mut()
            .fold(false, |changed, item| without_attachment_flags(item) | changed),
        _ => false,
    }
}
