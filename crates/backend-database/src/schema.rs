//! The schemas of the control database and of each conversation's database
//! (`storage.md`): one SQL text each, applied to a new database in a
//! transaction, whose digest is the version a database records. Times are
//! integer milliseconds since the Unix epoch; a closed set is text a CHECK
//! limits; JSON columns are text their reader decodes and validates; sealed
//! values are BLOBs.

use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::StorageError;

/// A database's schema: its SQL, whose digest names it.
pub(crate) struct Schema(&'static str);

pub(crate) const CONTROL: Schema = Schema(CONTROL_V1);

pub(crate) const CONVERSATION: Schema = Schema(CONVERSATION_V1);

impl Schema {
    /// The version a database of this schema records in `user_version`:
    /// the first 31 bits of its SQL's SHA-256, never 0, which SQLite gives a
    /// database that records none. Any edit of the SQL is another version.
    pub(crate) fn version(&self) -> i32 {
        let digest = Sha256::digest(self.0.as_bytes());
        let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) >> 1;
        let version = i32::try_from(bits).expect("31 bits fit");
        version.max(1)
    }

    /// Gives a new database at `path` this schema, and accepts one that
    /// records its version. A database that records another version, or
    /// holds tables and records none, was made by another build of Demi: it
    /// is refused, never changed (`storage.md` § Schemas).
    pub(crate) fn apply(
        &self,
        connection: &mut Connection,
        path: &Path,
    ) -> Result<(), StorageError> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recorded: i32 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if recorded == self.version() {
            return Ok(());
        }
        let tables: i64 = transaction.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
            [],
            |row| row.get(0),
        )?;
        if recorded != 0 || tables > 0 {
            return Err(StorageError::OtherSchema {
                path: path.to_owned(),
            });
        }
        transaction.execute_batch(self.0)?;
        transaction.pragma_update(None, "user_version", self.version())?;
        transaction.commit()?;
        Ok(())
    }
}

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

-- Each user's Subagent switch (`subagents.md` § Profiles): a user with no
-- row has subagents on.
CREATE TABLE user_subagents (
  user_id TEXT PRIMARY KEY REFERENCES users (id),
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1))
) STRICT;

-- Each user's subagent profiles: the model settings as JSON, null for the
-- parent's model, and the instructions that replace the parent's, null for
-- the parent's. A name is unique among its user's profiles.
CREATE TABLE subagent_profiles (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  description  TEXT NOT NULL,
  model        TEXT,
  instructions TEXT,
  can_spawn    INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  enabled      INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  UNIQUE (user_id, name)
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

-- Each choice a user made about a plugin (`plugins.md` § A user's plugins):
-- a plugin with no row is on.
CREATE TABLE user_plugins (
  user_id TEXT NOT NULL REFERENCES users (id),
  plugin  TEXT NOT NULL,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  PRIMARY KEY (user_id, plugin)
) STRICT, WITHOUT ROWID;

-- Each plugin's values for a user (`plugins.md` § The contract): a JSON
-- document its plugin decodes, the revision a conditional write names, and
-- the blobs the value names, a JSON array of their SHA-256s.
CREATE TABLE plugin_values (
  user_id  TEXT NOT NULL REFERENCES users (id),
  plugin   TEXT NOT NULL,
  key      TEXT NOT NULL,
  document TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  blobs    TEXT NOT NULL,
  PRIMARY KEY (user_id, plugin, key)
) STRICT, WITHOUT ROWID;

-- Each plugin's Host directories for a user (`plugins.md` § Host
-- directories): the directory's name, its digest and its files, a JSON
-- array of each file's path, whether it is executable and its SHA-256.
CREATE TABLE plugin_directories (
  user_id TEXT NOT NULL REFERENCES users (id),
  plugin  TEXT NOT NULL,
  name    TEXT NOT NULL,
  digest  TEXT NOT NULL,
  files   TEXT NOT NULL,
  PRIMARY KEY (user_id, plugin, name)
) STRICT, WITHOUT ROWID;

-- The conversation index. The web app chooses a conversation's id and its
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
  model               TEXT,
  user_messages       INTEGER NOT NULL,
  titled_messages     INTEGER NOT NULL,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  -- When the conversation's agent tree was last seen live, from which the
  -- retention pass reads how long the conversation has been idle.
  live_at             INTEGER NOT NULL,
  -- When the earliest yield wakeup its tree saved is due, which the next
  -- start restores the tree at: 0 for one whose action had not ended, due
  -- at start; null when it saved none.
  wakeup_at           INTEGER CHECK (wakeup_at >= 0),
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
CREATE INDEX conversations_wakeup ON conversations (wakeup_at) WHERE wakeup_at IS NOT NULL;

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

-- A conversation's work panel: its tabs and the ids it ever had, one JSON
-- document, with the revision that counts its changes.
CREATE TABLE conversation_panels (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 1),
  document        TEXT NOT NULL,
  updated_at      INTEGER NOT NULL
) STRICT;

-- A conversation's draft: its text and files with the revision they were
-- written at, and the version a save replaced with the revision that version
-- had. The row stays when the draft is emptied, so a revision is never used
-- twice.
CREATE TABLE conversation_drafts (
  conversation_id   TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision          INTEGER NOT NULL CHECK (revision >= 1),
  document          TEXT NOT NULL,
  written           INTEGER NOT NULL CHECK (written BETWEEN 1 AND revision),
  replaced_revision INTEGER,
  replaced          TEXT,
  updated_at        INTEGER NOT NULL,
  CHECK ((replaced IS NULL) = (replaced_revision IS NULL))
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

-- Providers: an API-key entry's sealed configuration, a subscription entry's
-- active account, a sealed secret per subscription account, and one
-- subscription entry per owner and family. The active account is not a
-- foreign key: removing an account clears it in the same transaction.
CREATE TABLE providers (
  id                   TEXT PRIMARY KEY,
  owner_user_id        TEXT NOT NULL REFERENCES users (id),
  provider_type        TEXT NOT NULL,
  credential_kind      TEXT NOT NULL CHECK (credential_kind IN ('api_key', 'subscription')),
  label                TEXT NOT NULL,
  config               BLOB,
  active_credential_id TEXT,
  created_at           INTEGER NOT NULL,
  CHECK ((credential_kind = 'api_key') = (config IS NOT NULL)),
  CHECK (credential_kind = 'subscription' OR active_credential_id IS NULL)
) STRICT;
CREATE INDEX providers_owner ON providers (owner_user_id, created_at);
CREATE UNIQUE INDEX providers_one_subscription ON providers (owner_user_id, provider_type)
  WHERE credential_kind = 'subscription';

CREATE TABLE provider_credentials (
  provider_id  TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  id           TEXT NOT NULL,
  identity_key TEXT,
  label        TEXT NOT NULL,
  detail       TEXT,
  source       TEXT NOT NULL,
  secret       BLOB NOT NULL,
  version      INTEGER NOT NULL CHECK (version >= 1),
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
  snippet    TEXT,
  created_at INTEGER NOT NULL
) STRICT;
";

/// One agent tree (`storage.md` § Conversation state and transactions): the
/// root node's id is the conversation's, and its descendants are subagents.
/// A node row carries identity and relationship; its checkpoint is the state
/// row's JSON and its block rows.
const CONVERSATION_V1: &str = r"
CREATE TABLE nodes (
  id               TEXT PRIMARY KEY,
  number           INTEGER NOT NULL UNIQUE CHECK (number >= 0),
  parent_id        TEXT REFERENCES nodes (id) ON DELETE CASCADE,
  description      TEXT NOT NULL,
  profile          TEXT,
  -- The instructions the node's profile replaced its parent's with; null
  -- when it has its parent's.
  instructions     TEXT,
  round            INTEGER NOT NULL CHECK (round >= 1),
  started_at       INTEGER NOT NULL,
  can_spawn        INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  closed_phase     TEXT CHECK (closed_phase IN ('completed', 'aborted', 'error')),
  closed_at        INTEGER,
  result           TEXT,
  failure          TEXT,
  delivered        INTEGER NOT NULL CHECK (delivered IN (0, 1)),
  -- The session's checkpoint state as JSON, with whether a Stop holds its
  -- waiting input until the user's next action.
  state            TEXT NOT NULL,
  block_count      INTEGER NOT NULL CHECK (block_count >= 0),
  output_revision  INTEGER NOT NULL CHECK (output_revision >= 0),
  -- When the earliest wakeup the state saves is due, as the index of
  -- conversations holds it; null when it saves none.
  wakeup_at        INTEGER CHECK (wakeup_at >= 0),
  CHECK ((closed_phase IS NULL) = (closed_at IS NULL)),
  CHECK (result IS NULL OR closed_phase = 'completed'),
  CHECK (failure IS NULL OR closed_phase = 'error')
) STRICT;
-- One root per tree.
CREATE UNIQUE INDEX nodes_root ON nodes ((parent_id IS NULL)) WHERE parent_id IS NULL;
CREATE INDEX nodes_children ON nodes (parent_id, number);

-- The next number of each sequence the model sees in the conversation: a
-- number is advanced past in its own transaction before it is given out,
-- so a crash leaves a gap and never gives a number twice.
CREATE TABLE sequences (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'shell', 'agent', 'tab')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

CREATE TABLE blocks (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx     INTEGER NOT NULL CHECK (idx >= 0),
  block   TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
) STRICT;

-- An index of the blobs the blocks reference, one row per reference, media
-- and edit copies: derived from each block in the transaction that writes
-- it, never written on its own. The retention pass finds expired tool media
-- and referenced blobs here, which SQLite cannot index inside a block's JSON.
CREATE TABLE blob_refs (
  node_id TEXT NOT NULL,
  idx     INTEGER NOT NULL,
  part    INTEGER NOT NULL CHECK (part >= 0),
  blob    TEXT NOT NULL,
  holder  TEXT NOT NULL CHECK (holder IN ('message', 'tool_result', 'edit_copy')),
  at      INTEGER NOT NULL,
  PRIMARY KEY (node_id, idx, part),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT;
CREATE INDEX blob_refs_expiry ON blob_refs (holder, at);

-- Each ended command's whole output: stored as a blob, with the bytes at
-- its end the backend does not have and why; not stored, and why; or
-- removed by the retention pass, and when.
CREATE TABLE command_outputs (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  -- The command's media, a JSON array, beside a stored output.
  media          TEXT,
  not_stored     TEXT,
  removed_at     INTEGER,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) + (removed_at IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL),
  CHECK ((media IS NULL) = (blob IS NULL))
) STRICT;
CREATE INDEX command_outputs_expiry ON command_outputs (ended_at) WHERE blob IS NOT NULL;
";
