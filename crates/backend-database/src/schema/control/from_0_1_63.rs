//! From 0.1.63's control schema, which 0.1.49 to 0.1.62 shipped too, to
//! 0.1.65's.
//!
//! - A plugin keeps what its package on a Host last reported of a
//!   conversation (`plugin_reports`), which its page reads at once; nothing
//!   was kept before, so the next report fills it.
//! - A detach applies at once, so no host keeps a mark to detach once its
//!   conversation's work ends. A detach an agent asked for before the upgrade
//!   is made here, as the commit of a detach makes it: the host leaves the
//!   conversation's, and the conversation's hosts and execution context move
//!   on. Each host's runner lets go of the conversation at its next `hello`,
//!   which names only the conversations it is to hold.
//! - A draft is the last one saved: its row no longer keeps the revision its
//!   text was written at nor the version a save replaced. A replaced version
//!   is dropped, its files with it, since the blobs a draft names are read
//!   from its row. The table is rebuilt, since its checks name those columns.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_63);

const CONTROL_FROM_0_1_63: &str = "
UPDATE conversations
   SET context_version = context_version + 1, hosts_revision = hosts_revision + 1
 WHERE id IN (SELECT conversation_id FROM conversation_hosts WHERE detaching = 1);
DELETE FROM conversation_hosts WHERE detaching = 1;
ALTER TABLE conversation_hosts DROP COLUMN detaching;
CREATE TABLE conversation_drafts_next (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 1),
  document        TEXT NOT NULL,
  saved_revision  INTEGER NOT NULL CHECK (saved_revision >= 0),
  updated_at      INTEGER NOT NULL
) STRICT;
INSERT INTO conversation_drafts_next (conversation_id, revision, document, saved_revision, updated_at)
  SELECT conversation_id, revision, document, saved_revision, updated_at FROM conversation_drafts;
DROP TABLE conversation_drafts;
ALTER TABLE conversation_drafts_next RENAME TO conversation_drafts;
CREATE TABLE plugin_reports (
  plugin_id       TEXT NOT NULL,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 0),
  value           TEXT NOT NULL,
  reported_at     INTEGER NOT NULL,
  PRIMARY KEY (plugin_id, conversation_id, device_id)
) STRICT;
";
