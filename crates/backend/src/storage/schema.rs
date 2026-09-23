//! The schemas of the control database and of each conversation's database:
//! one versioned step each, applied to a new database in a transaction
//! (`storage.md`). Times are integer milliseconds since the Unix epoch; a
//! closed set is text a CHECK limits; JSON columns are text their reader
//! decodes and validates; sealed values are BLOBs.

use rusqlite_migration::{M, Migrations};

pub(super) const CONTROL: Migrations<'static> = Migrations::from_slice(CONTROL_STEPS);

const CONTROL_STEPS: &[M<'static>] = &[M::up(CONTROL_V1)];

pub(super) const CONVERSATION: Migrations<'static> = Migrations::from_slice(CONVERSATION_STEPS);

const CONVERSATION_STEPS: &[M<'static>] = &[M::up(CONVERSATION_V1)];

const CONTROL_V1: &str = r"
-- Identity. One master: two concurrent setups cannot both create one.
CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
  nickname      TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  role          TEXT NOT NULL CHECK (role IN ('master', 'admin', 'user')),
  created_at    INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX users_one_master ON users (role) WHERE role = 'master';

CREATE TABLE web_sessions (
  token_hash TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  expires_at INTEGER NOT NULL
) STRICT;
CREATE INDEX web_sessions_expiry ON web_sessions (expires_at);

-- One pending address change per user.
CREATE TABLE email_challenges (
  user_id       TEXT PRIMARY KEY REFERENCES users (id),
  id            TEXT NOT NULL UNIQUE,
  email         TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  code_hash     TEXT NOT NULL,
  expires_at    INTEGER NOT NULL,
  sent_at       INTEGER NOT NULL,
  attempts      INTEGER NOT NULL
) STRICT;

CREATE TABLE user_preferences (
  user_id     TEXT PRIMARY KEY REFERENCES users (id),
  preferences TEXT NOT NULL
) STRICT;

-- Devices and workspaces. The token hash is absent until the backend issues
-- a token; one managed device per user.
CREATE TABLE devices (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  kind         TEXT NOT NULL CHECK (kind IN ('user', 'managed')),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
  token_hash   TEXT UNIQUE,
  claimed_at   INTEGER NOT NULL,
  last_seen_at INTEGER
) STRICT;
CREATE UNIQUE INDEX devices_one_managed ON devices (user_id) WHERE kind = 'managed';

CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  device_id  TEXT NOT NULL REFERENCES devices (id),
  path       TEXT NOT NULL,
  name       TEXT NOT NULL,
  sort_order INTEGER NOT NULL,
  created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX workspaces_order ON workspaces (user_id, sort_order);

CREATE TABLE exposes (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  device_id  TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  address    TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
) STRICT;
CREATE INDEX exposes_expiry ON exposes (expires_at);
CREATE INDEX exposes_owner ON exposes (user_id, expires_at);

-- The conversation index. The browser chooses a conversation's id and its
-- case is kept, but no two ids differ only in case: each names a database
-- file, and a file system may ignore case. The target is Cloud (optionally a
-- directory on it), a device directory or a workspace, checked per kind. The
-- target columns are not foreign keys: a conversation that targets a device
-- directly does not block revoking that device.
CREATE TABLE conversations (
  id                  TEXT PRIMARY KEY COLLATE NOCASE,
  user_id             TEXT NOT NULL REFERENCES users (id),
  title               TEXT NOT NULL,
  title_origin        TEXT NOT NULL
                        CHECK (title_origin IN ('placeholder', 'message', 'generated', 'user')),
  archived            INTEGER NOT NULL CHECK (archived IN (0, 1)),
  pinned              INTEGER NOT NULL CHECK (pinned IN (0, 1)),
  sort_order          INTEGER NOT NULL,
  read_revision       INTEGER NOT NULL,
  target_kind         TEXT NOT NULL CHECK (target_kind IN ('cloud', 'device', 'workspace')),
  target_device_id    TEXT,
  target_path         TEXT,
  target_workspace_id TEXT,
  last_switch         TEXT,
  cloud_reset_id      TEXT,
  context_version     INTEGER NOT NULL,
  provider_id         TEXT,
  model_id            TEXT,
  user_messages       INTEGER NOT NULL,
  titled_messages     INTEGER NOT NULL,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  CHECK (
    (target_kind = 'cloud'
      AND target_device_id IS NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'device'
      AND target_device_id IS NOT NULL AND target_path IS NOT NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'workspace'
      AND target_workspace_id IS NOT NULL AND target_device_id IS NULL AND target_path IS NULL)
  )
) STRICT;
CREATE INDEX conversations_sidebar ON conversations (user_id, archived, pinned, sort_order);
CREATE INDEX conversations_workspace ON conversations (target_workspace_id);

CREATE TABLE conversation_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id),
  device_id       TEXT NOT NULL REFERENCES devices (id),
  name            TEXT NOT NULL,
  cwd             TEXT,
  attached_at     INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, device_id),
  UNIQUE (conversation_id, name)
) STRICT;
CREATE INDEX conversation_hosts_device ON conversation_hosts (device_id);

CREATE TABLE conversation_panels (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  document        TEXT NOT NULL,
  updated_at      INTEGER NOT NULL
) STRICT;

-- Records that make interrupted multi-step work discoverable. A Fork
-- reserves its destination's conversation id, compared as the index
-- compares it.
CREATE TABLE conversation_fork_operations (
  id        TEXT PRIMARY KEY COLLATE NOCASE,
  user_id   TEXT NOT NULL REFERENCES users (id),
  source_id TEXT NOT NULL,
  block_id  TEXT NOT NULL,
  metadata  TEXT NOT NULL
) STRICT;

CREATE TABLE managed_operations (
  device_id    TEXT NOT NULL REFERENCES devices (id),
  operation_id TEXT NOT NULL,
  base_version TEXT NOT NULL,
  phase        TEXT NOT NULL
                 CHECK (phase IN ('stopping', 'saving', 'rebuilding', 'booting', 'ready', 'failed')),
  error        TEXT,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (device_id, operation_id)
) STRICT;

-- Providers: a sealed configuration per entry, a sealed secret per
-- subscription account, one subscription entry per owner and family.
CREATE TABLE providers (
  id                   TEXT PRIMARY KEY,
  owner_user_id        TEXT NOT NULL REFERENCES users (id),
  provider_type        TEXT NOT NULL,
  credential_kind      TEXT NOT NULL CHECK (credential_kind IN ('api_key', 'subscription')),
  label                TEXT NOT NULL,
  config               BLOB NOT NULL,
  active_credential_id TEXT,
  created_at           INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX providers_one_subscription ON providers (owner_user_id, provider_type)
  WHERE credential_kind = 'subscription';

CREATE TABLE provider_credentials (
  provider_id  TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  id           TEXT NOT NULL,
  identity_key TEXT,
  label        TEXT NOT NULL,
  detail       TEXT,
  source       TEXT,
  secret       BLOB NOT NULL,
  version      INTEGER NOT NULL,
  quota        TEXT,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (provider_id, id)
) STRICT;
CREATE UNIQUE INDEX provider_credentials_identity ON provider_credentials (provider_id, identity_key)
  WHERE identity_key IS NOT NULL;

CREATE TABLE model_catalogs (
  provider_id TEXT PRIMARY KEY REFERENCES providers (id) ON DELETE CASCADE,
  record      TEXT NOT NULL
) STRICT;

-- Usage and attachments; neither holds attachment bytes.
CREATE TABLE usage_ledger (
  id                 TEXT PRIMARY KEY,
  user_id            TEXT NOT NULL REFERENCES users (id),
  conversation_id    TEXT NOT NULL,
  provider_id        TEXT NOT NULL,
  model_id           TEXT NOT NULL,
  input_tokens       INTEGER NOT NULL,
  output_tokens      INTEGER NOT NULL,
  cache_read_tokens  INTEGER NOT NULL,
  cache_write_tokens INTEGER NOT NULL,
  created_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, created_at);

CREATE TABLE attachments (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  media_type TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  sha256     TEXT NOT NULL,
  created_at INTEGER NOT NULL
) STRICT;
";

/// One agent tree (`storage.md` § Conversation state and transactions): the
/// root node's id is the conversation's, and its descendants are subagents.
/// A node row carries identity and relationship; its checkpoint is the state
/// row's JSON, its block rows and its command state, whose versions are
/// immutable and whose boundaries each name a version.
const CONVERSATION_V1: &str = r"
CREATE TABLE nodes (
  id               TEXT PRIMARY KEY,
  parent_id        TEXT REFERENCES nodes (id) ON DELETE CASCADE,
  description      TEXT NOT NULL,
  profile          TEXT,
  spawned_at       INTEGER NOT NULL,
  can_spawn        INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  closed_phase     TEXT CHECK (closed_phase IN ('completed', 'aborted', 'error')),
  closed_at        INTEGER,
  result           TEXT,
  failure          TEXT,
  delivered        INTEGER NOT NULL CHECK (delivered IN (0, 1)),
  state            TEXT NOT NULL,
  block_count      INTEGER NOT NULL CHECK (block_count >= 0),
  command_revision INTEGER NOT NULL CHECK (command_revision >= 0),
  output_revision  INTEGER NOT NULL CHECK (output_revision >= 0),
  CHECK ((closed_phase IS NULL) = (closed_at IS NULL)),
  CHECK (result IS NULL OR closed_phase = 'completed'),
  CHECK (failure IS NULL OR closed_phase = 'error')
) STRICT;
-- One root per tree.
CREATE UNIQUE INDEX nodes_root ON nodes ((parent_id IS NULL)) WHERE parent_id IS NULL;
CREATE INDEX nodes_children ON nodes (parent_id, spawned_at);

CREATE TABLE blocks (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx     INTEGER NOT NULL CHECK (idx >= 0),
  block   TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
) STRICT;

-- Each version holds the node's complete command-storage map.
CREATE TABLE command_snapshots (
  node_id  TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK (revision >= 0),
  entries  TEXT NOT NULL,
  PRIMARY KEY (node_id, revision)
) STRICT;

-- A boundary names the version that was current at its edge of a block; a
-- version a boundary names cannot go.
CREATE TABLE session_boundaries (
  node_id          TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  block_id         TEXT NOT NULL,
  edge             TEXT NOT NULL CHECK (edge IN ('before_user', 'after_assistant', 'after_block')),
  command_revision INTEGER NOT NULL,
  PRIMARY KEY (node_id, block_id, edge),
  FOREIGN KEY (node_id, command_revision) REFERENCES command_snapshots (node_id, revision)
) STRICT;
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_schema_applies_to_a_new_database() {
        CONTROL.validate().unwrap();
        CONVERSATION.validate().unwrap();
    }
}
