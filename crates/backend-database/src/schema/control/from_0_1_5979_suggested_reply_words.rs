//! From the control schema whose suggested replies are words: a device is
//! allowed one at a time. A request names the device it brings in, and the
//! grant of all of the user's devices, `host.use`, goes with its undecided
//! requests; a decided one stays until its message is delivered.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_SUGGESTED_REPLY_WORDS);

const CONTROL_FROM_0_1_5979_SUGGESTED_REPLY_WORDS: &str = "
CREATE TABLE permission_requests_next (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('permission', 'device', 'credential')),
  categories        TEXT,
  device_id         TEXT,
  credential_name   TEXT,
  ask               TEXT,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  decision          TEXT CHECK (decision IN ('allowed', 'denied', 'reached')),
  decided_at        INTEGER,
  read              INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1)),
  CHECK ((kind = 'permission') = (categories IS NOT NULL)),
  CHECK ((kind = 'device') = (device_id IS NOT NULL)),
  CHECK ((kind = 'credential') = (credential_name IS NOT NULL)),
  CHECK ((kind = 'credential') = (ask IS NOT NULL)),
  CHECK (kind <> 'credential' OR decision IS NULL),
  CHECK ((agent_number IS NULL) = (agent_description IS NULL)),
  CHECK ((decision IS NULL) = (decided_at IS NULL))
) STRICT;
INSERT INTO permission_requests_next
  (id, conversation_id, kind, categories, credential_name, ask, command, node_id,
   agent_number, agent_description, created_at, decision, decided_at, read)
  SELECT id, conversation_id, kind, categories, credential_name, ask, command, node_id,
         agent_number, agent_description, created_at, decision, decided_at, read
  FROM permission_requests
  WHERE kind <> 'permission' OR categories <> '[\"host.use\"]' OR decision IS NOT NULL;
DROP TABLE permission_requests;
ALTER TABLE permission_requests_next RENAME TO permission_requests;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);
DELETE FROM permission_grants WHERE category = 'host.use';
";
