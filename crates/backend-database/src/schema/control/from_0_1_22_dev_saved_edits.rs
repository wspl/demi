//! From the control schema that kept the edits and sends of saved messages
//! to the one where an attached host goes by its device's name
//! (`crates/backend-database/src/conversation_index.rs`) and a switch is known by the
//! revision it made.
//!
//! The hosts' table loses the names conversations gave them, rebuilt since SQLite
//! drops no column a constraint names, with the rows in the order the names were
//! listed in, which their row ids keep.
//!
//! A Fork's metadata goes from format 1, whose attached hosts carry names, to
//! format 2, which holds the source's execution-context revision at the Fork
//! instead: each record is decoded as format 1 and the result checked as format 2,
//! so a record of neither stops the migration.
//!
//! - A reservation stays once its destination is published, which never reads it
//!   again, and keeps the destination's revision.
//! - One that waits takes its source's revision now, at least the one at the Fork
//!   and so as good.
//! - One whose source is gone too, deleted while the Fork waited, takes 0: none is
//!   known, and the cost, a first switch of the destination that its copied blocks
//!   hide, is the one a switch made before the upgrade takes, below.
//!
//! A switch made before records revision 0, which every node read or passed, since
//! the revision it made is not known: the conversation's current one may have
//! moved past it while its nodes were idle, and would announce a switch they saw
//! once more. So a switch made while its nodes were idle that none of them had read
//! yet loses its one announcement, which is accepted: the conversation's
//! transcript shows the user the move.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob, corrupt, corrupt_text};
use super::shared::{fork_format_2};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_saved_edits);

fn control_from_saved_edits(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
CREATE TABLE conversation_hosts_next (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id),
  device_id       TEXT NOT NULL REFERENCES devices (id),
  cwd             TEXT,
  attached_at     INTEGER NOT NULL,
  detaching       INTEGER NOT NULL DEFAULT 0 CHECK (detaching IN (0, 1)),
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
INSERT INTO conversation_hosts_next (conversation_id, device_id, cwd, attached_at, detaching)
  SELECT conversation_id, device_id, cwd, attached_at, detaching FROM conversation_hosts
  ORDER BY attached_at, name;
DROP TABLE conversation_hosts;
ALTER TABLE conversation_hosts_next RENAME TO conversation_hosts;
CREATE INDEX conversation_hosts_device ON conversation_hosts (device_id);
ALTER TABLE conversations ADD COLUMN last_switch_revision INTEGER;
UPDATE conversations SET last_switch_revision = 0 WHERE last_switch IS NOT NULL;
",
    )?;
    let records = transaction
        .prepare(
            "SELECT fork.id, fork.metadata,
               coalesce(destination.context_version, source.context_version, 0)
             FROM conversation_fork_operations fork
               LEFT JOIN conversations destination ON destination.id = fork.id
               LEFT JOIN conversations source ON source.id = fork.source_id",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE conversation_fork_operations SET metadata = ?2 WHERE id = ?1")?;
    for (id, text, revision) in records {
        let context_version = u64::try_from(revision).map_err(corrupt)?;
        let named: fork_format_1::ForkMetadata = serde_json::from_str(&text).map_err(corrupt)?;
        if named.title.is_empty() || named.attached_hosts.iter().any(|host| host.name.is_empty()) {
            return Err(corrupt_text("a fork's metadata of format 1 has an empty name"));
        }
        let metadata = fork_format_2::ForkMetadata {
            title: Some(named.title),
            target: named.target,
            model: named.model,
            created_at: named.created_at,
            attached_hosts: named
                .attached_hosts
                .into_iter()
                .map(|host| fork_format_2::AttachedHost {
                    device: host.device,
                    cwd: host.cwd,
                })
                .collect(),
            context_version,
        };
        let text = serde_json::to_string(&metadata).map_err(corrupt)?;
        update.execute(rusqlite::params![id, text])?;
    }
    Ok(())
}

/// Fork metadata format 1, which named each attached host, as the control
/// schema before [`control_from_saved_edits`] stored it.
mod fork_format_1 {
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct ForkMetadata {
        pub title: String,
        /// The target as the release that stored it wrote it.
        pub target: serde_json::Value,
        #[serde(deserialize_with = "Option::deserialize")]
        pub model: Option<serde_json::Value>,
        pub created_at: serde_json::Value,
        pub attached_hosts: Vec<AttachedHost>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct AttachedHost {
        pub device: String,
        /// Format 1's name, which format 2 drops: decoded and checked so a
        /// record without one is not taken for format 1.
        pub name: String,
        #[serde(deserialize_with = "Option::deserialize")]
        pub cwd: Option<String>,
    }
}
