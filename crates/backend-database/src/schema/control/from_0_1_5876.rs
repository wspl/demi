//! From 0.1.5876's control schema: a permission covers a whole capability.
//!
//! - A conversation that may use one of the user's credentials, or that holds
//!   the grant to manage them, gets the grant of the user's credentials,
//!   `credentials.use`.
//! - A conversation that may reach a device it would not reach without the
//!   grant of the user's devices, that is other than its primary Host, its
//!   user's Cloud for a conversation there and a device attached to it, or that
//!   holds the grant to manage port forwards, gets that grant, `host.use`.
//! - The grant to read the user's other conversations, and the one to message
//!   them an earlier release left, become the grant of the user's other
//!   conversations, `conversation.use`.
//! - A request of one of the categories that go asks for the one that covers
//!   it now. A request to use a single credential, to reach a single device or
//!   to pair a single computer goes, decided or not, with the lists those
//!   allowed. An undecided request of a category the conversation now holds is
//!   allowed, so that its agent is told at the next start, as an allow tells
//!   it.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5876);

const CONTROL_FROM_0_1_5876: &str = "
INSERT OR IGNORE INTO permission_grants (conversation_id, category, granted_at)
  SELECT conversation_id, 'credentials.use', min(allowed_at) FROM credential_allowances GROUP BY conversation_id;
INSERT OR IGNORE INTO permission_grants (conversation_id, category, granted_at)
  SELECT r.conversation_id, 'host.use', min(r.added_at)
  FROM reachable_hosts r JOIN conversations c ON c.id = r.conversation_id
  WHERE c.target_device_id IS NOT r.device_id
    AND NOT (c.target_kind = 'cloud' AND EXISTS (
      SELECT 1 FROM devices d
      WHERE d.id = r.device_id AND d.kind = 'managed' AND d.machine = 'cloud' AND d.user_id = c.user_id))
    AND NOT EXISTS (
      SELECT 1 FROM conversation_hosts h WHERE h.conversation_id = r.conversation_id AND h.device_id = r.device_id)
  GROUP BY r.conversation_id;
INSERT OR IGNORE INTO permission_grants (conversation_id, category, granted_at)
  SELECT conversation_id,
         CASE category
           WHEN 'credentials.manage' THEN 'credentials.use'
           WHEN 'host.forward' THEN 'host.use'
           ELSE 'conversation.use'
         END,
         granted_at
  FROM permission_grants
  WHERE category IN ('credentials.manage', 'host.forward', 'conversation.read', 'conversation.send');
DELETE FROM permission_grants
  WHERE category IN ('credentials.manage', 'host.forward', 'conversation.read', 'conversation.send');

DELETE FROM permission_requests
  WHERE kind = 'allowance' OR host_device_id IS NOT NULL OR pairing_code IS NOT NULL;
UPDATE permission_requests SET categories = '[\"credentials.use\"]' WHERE categories = '[\"credentials.manage\"]';
UPDATE permission_requests SET categories = '[\"host.use\"]' WHERE categories = '[\"host.forward\"]';
UPDATE permission_requests SET categories = '[\"conversation.use\"]'
  WHERE categories IN ('[\"conversation.read\"]', '[\"conversation.send\"]');
UPDATE permission_requests SET decision = 'allowed', decided_at = created_at
  WHERE kind = 'permission' AND decision IS NULL AND EXISTS (
    SELECT 1 FROM permission_grants g
    WHERE g.conversation_id = permission_requests.conversation_id
      AND g.category = json_extract(permission_requests.categories, '$[0]'));

CREATE TABLE permission_requests_next (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('permission', 'credential')),
  categories        TEXT,
  credential_name   TEXT,
  ask               TEXT,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  decision          TEXT CHECK (decision IN ('allowed', 'denied')),
  decided_at        INTEGER,
  read              INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1)),
  CHECK ((kind = 'permission') = (categories IS NOT NULL)),
  CHECK ((kind = 'permission') = (credential_name IS NULL)),
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
  FROM permission_requests;
DROP TABLE permission_requests;
ALTER TABLE permission_requests_next RENAME TO permission_requests;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);

DROP TABLE reachable_hosts;
DROP TABLE pairing_passes;
DROP TABLE credential_allowances;
";
