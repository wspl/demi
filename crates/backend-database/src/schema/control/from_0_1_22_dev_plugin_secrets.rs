//! From the control schema with the plugins' secrets to the one with the users'
//! credentials.
//!
//! - Two new tables, empty.
//! - A conversation's requests of three kinds, of which every stored one is a
//!   permission request.
//!
//! The requests' table is rebuilt, since SQLite cannot make a column nullable in
//! place.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_PLUGIN_SECRETS);

const CONTROL_FROM_PLUGIN_SECRETS: &str = "
CREATE TABLE user_credentials (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  note         TEXT NOT NULL,
  fields       TEXT NOT NULL,
  sealed       BLOB NOT NULL,
  keep_until   INTEGER,
  last_used_at INTEGER,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  UNIQUE (user_id, name)
) STRICT;
CREATE INDEX user_credentials_by_expiry ON user_credentials (keep_until) WHERE keep_until IS NOT NULL;
CREATE TABLE permission_requests_next (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('permission', 'credential', 'allowance')),
  categories        TEXT,
  credential_name   TEXT,
  credential_id     TEXT REFERENCES user_credentials (id) ON DELETE SET NULL,
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
  CHECK (kind = 'allowance' OR credential_id IS NULL),
  CHECK (kind <> 'credential' OR decision IS NULL),
  CHECK ((agent_number IS NULL) = (agent_description IS NULL)),
  CHECK ((decision IS NULL) = (decided_at IS NULL))
) STRICT;
INSERT INTO permission_requests_next
  (id, conversation_id, kind, categories, command, node_id, agent_number, agent_description,
   created_at, decision, decided_at, read)
  SELECT id, conversation_id, 'permission', categories, command, node_id, agent_number,
         agent_description, created_at, decision, decided_at, read
  FROM permission_requests;
DROP TABLE permission_requests;
ALTER TABLE permission_requests_next RENAME TO permission_requests;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);
CREATE TABLE credential_allowances (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  credential_id   TEXT NOT NULL REFERENCES user_credentials (id) ON DELETE CASCADE,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, credential_id)
) STRICT;
CREATE INDEX credential_allowances_of_credential ON credential_allowances (credential_id);
";
