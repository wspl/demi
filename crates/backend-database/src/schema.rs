//! The schemas of the control database and of each conversation's database
//! (`storage.md` § Schemas and migrations): one SQL text each, applied to a
//! new database in a transaction, whose digest is the version a database
//! records, and the history of the schemas published releases shipped
//! before it, each with the migration to the next. Times are integer
//! milliseconds since the Unix epoch; a closed set is text a CHECK limits;
//! JSON columns are text their reader decodes and validates; sealed values
//! are BLOBs.

use std::path::Path;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::StorageError;

/// A database's schema: its SQL, whose digest names it, and the schemas
/// that published releases shipped before it, oldest first.
pub(crate) struct Schema {
    sql: &'static str,
    history: &'static [Shipped],
}

/// A schema a published release shipped, and the migration from it to the
/// schema after it in the history, or to the current one after the last.
pub(crate) struct Shipped {
    sql: &'static str,
    migration: Migration,
}

/// What turns a database of one schema into one of the next.
pub(crate) enum Migration {
    Sql(&'static str),
    /// A change SQL cannot express, such as re-encoding a stored value.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "every migration so far is SQL")
    )]
    Code(fn(&Transaction<'_>) -> rusqlite::Result<()>),
}

/// The control database's. Its history holds the schema of each published
/// release before the one that ships the current schema; 0.1.14 and 0.1.15
/// shipped the one of 0.1.15, and 0.1.16 the last one in it.
pub(crate) const CONTROL: Schema = Schema {
    sql: CONTROL_V1,
    history: &[
        Shipped {
            sql: include_str!("schema/control-0.1.11.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_11),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.13.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_13),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.15.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_15),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.16.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_16),
        },
    ],
};

/// Each conversation's database's. Its history holds the schema of each
/// published release before the one that shipped the current schema; 0.1.12
/// shipped it.
pub(crate) const CONVERSATION: Schema = Schema {
    sql: CONVERSATION_V1,
    history: &[Shipped {
        sql: include_str!("schema/conversation-0.1.11.sql"),
        migration: Migration::Sql(CONVERSATION_FROM_0_1_11),
    }],
};

/// From 0.1.11's control schema: the retention pass, which read when each
/// conversation was last live, is gone, and the column with it.
const CONTROL_FROM_0_1_11: &str = "
ALTER TABLE conversations DROP COLUMN live_at;
";

/// From 0.1.13's control schema: Host expose is gone, and its records with
/// it; a device keeps the operating system its runner reports, which a
/// device migrated from 0.1.13 learns at its runner's next hello.
const CONTROL_FROM_0_1_13: &str = "
DROP TABLE exposes;
ALTER TABLE devices ADD COLUMN os TEXT;
";

/// From 0.1.15's control schema: a conversation counts the changes of its
/// attached hosts, from 0.
const CONTROL_FROM_0_1_15: &str = "
ALTER TABLE conversations ADD COLUMN hosts_revision INTEGER NOT NULL DEFAULT 0 CHECK (hosts_revision >= 0);
";

/// From 0.1.16's control schema: a conversation's deletion is recorded as
/// pending until its last step (`storage.md` § Deleting a conversation).
const CONTROL_FROM_0_1_16: &str = "
CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  user_id    TEXT NOT NULL REFERENCES users (id),
  deleted_at INTEGER NOT NULL
) STRICT;
";

/// From 0.1.11's conversation schema. SQLite cannot change a table's CHECK
/// or drop a column a CHECK names, so such a table is made anew under
/// another name, its rows are copied, and it takes the old one's place
/// (SQLite's `ALTER TABLE`, § Making Other Kinds Of Table Schema Changes).
const CONVERSATION_FROM_0_1_11: &str = "
-- The retention pass and the blob collector are gone, and with them
-- blob_refs, an index derived from the blocks that holds nothing else.
DROP TABLE blob_refs;

-- The attachment sequence.
CREATE TABLE sequences_next (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'shell', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;
INSERT INTO sequences_next (name, next) SELECT name, next FROM sequences;
DROP TABLE sequences;
ALTER TABLE sequences_next RENAME TO sequences;

-- An output is no longer removed: the removed_at column goes, and the index
-- the retention pass searched. A row of an output 0.1.11 removed has no
-- state here, so its copy fails the CHECK, and with it the migration, which
-- leaves the database as 0.1.11 left it.
DROP INDEX command_outputs_expiry;
CREATE TABLE command_outputs_next (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  media          TEXT,
  not_stored     TEXT,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL),
  CHECK ((media IS NULL) = (blob IS NULL))
) STRICT;
INSERT INTO command_outputs_next
  (command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored)
  SELECT command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored
  FROM command_outputs;
DROP TABLE command_outputs;
ALTER TABLE command_outputs_next RENAME TO command_outputs;

-- The attachments the agent uploads.
CREATE TABLE attachments (
  number     INTEGER PRIMARY KEY CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL))
) STRICT;
";

/// A kind of database, as a server's upgrade asks about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Control,
    Conversation,
}

/// Whether this build would change the database at `path` as it opens it:
/// one that records another schema version than this build's, which it
/// migrates or refuses (`upgrades.md` § Prepare). The database is read, never
/// written.
pub fn schema_differs(path: &Path, kind: DatabaseKind) -> Result<bool, StorageError> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let recorded: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let schema = match kind {
        DatabaseKind::Control => &CONTROL,
        DatabaseKind::Conversation => &CONVERSATION,
    };
    Ok(recorded != schema.version())
}

/// The version a database of the schema `sql` records in `user_version`:
/// the first 31 bits of its SHA-256, never 0, which SQLite gives a database
/// that records none. Any edit of the SQL is another version.
fn version_of(sql: &str) -> i32 {
    let digest = Sha256::digest(sql.as_bytes());
    let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) >> 1;
    let version = i32::try_from(bits).expect("31 bits fit");
    version.max(1)
}

impl Schema {
    pub(crate) fn version(&self) -> i32 {
        version_of(self.sql)
    }

    /// The migrations that bring a database of `recorded`'s schema to this
    /// one, in order; none for a version the history does not hold.
    fn migrations(&self, recorded: i32) -> Option<&'static [Shipped]> {
        let from = self
            .history
            .iter()
            .position(|shipped| version_of(shipped.sql) == recorded)?;
        Some(&self.history[from..])
    }

    /// Gives a new database at `path` this schema, migrates one of a schema
    /// in the history, and accepts one that records this schema's version,
    /// each in one transaction. Any other database was made by a newer
    /// release or another build of Demi: it is refused, never changed.
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
        if recorded == 0 && tables == 0 {
            transaction.execute_batch(self.sql)?;
        } else {
            let migrations = self
                .migrations(recorded)
                .ok_or_else(|| StorageError::OtherSchema {
                    path: path.to_owned(),
                })?;
            for shipped in migrations {
                let migrated = match shipped.migration {
                    Migration::Sql(sql) => transaction.execute_batch(sql),
                    Migration::Code(migrate) => migrate(&transaction),
                };
                migrated.map_err(|source| StorageError::Migration {
                    path: path.to_owned(),
                    from: version_of(shipped.sql),
                    source,
                })?;
            }
        }
        transaction.pragma_update(None, "user_version", self.version())?;
        transaction.commit()?;
        Ok(())
    }

    /// Whether a database that records `recorded` opens as it is, or must be
    /// migrated by `apply` first; `None` for one this build cannot open.
    pub(crate) fn current(&self, recorded: i32) -> Option<bool> {
        if recorded == self.version() {
            return Some(true);
        }
        self.migrations(recorded).map(|_| false)
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
  last_seen_at INTEGER,
  installed    TEXT NOT NULL DEFAULT '[]',
  -- The operating system its runner last reported, JSON; none before its
  -- runner first connected.
  os           TEXT
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
  -- When the earliest yield wakeup its tree saved is due, which the next
  -- start restores the tree at: 0 for one whose action had not ended, due
  -- at start; null when it saved none.
  wakeup_at           INTEGER CHECK (wakeup_at >= 0),
  -- The revision of its attached hosts, raised by each change of them and
  -- of the directory a host's shell recorded; 0 before the first.
  hosts_revision      INTEGER NOT NULL DEFAULT 0 CHECK (hosts_revision >= 0),
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

-- Conversation permissions (permissions.md): each command an agent ran
-- without the grant of its category, undecided until the user decides and
-- then kept until the decision's message is in the agent's checkpoint; and
-- each category the user allowed for a conversation. The agent is the root
-- when its number and description are null.
CREATE TABLE permission_requests (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  category          TEXT NOT NULL,
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
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);

CREATE TABLE permission_grants (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  category        TEXT NOT NULL,
  granted_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, category)
) STRICT;

-- A conversation's deletion that has not reached its last step: its record
-- is gone, and a start finishes what is left (`storage.md` § Deleting a
-- conversation). The id is compared as the index compares it.
CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  user_id    TEXT NOT NULL REFERENCES users (id),
  deleted_at INTEGER NOT NULL
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
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'shell', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

CREATE TABLE blocks (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx     INTEGER NOT NULL CHECK (idx >= 0),
  block   TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
) STRICT;

-- Each ended command's whole output: stored as a blob, with the bytes at
-- its end the backend does not have and why; or not stored, and why.
CREATE TABLE command_outputs (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  -- The command's media, a JSON array, beside a stored output.
  media          TEXT,
  not_stored     TEXT,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL),
  CHECK ((media IS NULL) = (blob IS NULL))
) STRICT;

-- Each attachment the agent uploaded, by its number in the conversation's
-- attachment sequence: the file's name, the media type read from its bytes,
-- its size, an image's or a video's width and height in pixels when its
-- header gives them, and its blob in the owner's namespace. A row never
-- changes; a Fork's seed copies the rows, and the blobs stay shared.
CREATE TABLE attachments (
  number     INTEGER PRIMARY KEY CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL))
) STRICT;
";

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables, columns, indexes and foreign keys of a database, as
    /// SQLite reports them, and each table's and index's definition with
    /// its constraints, whatever SQL made them: comments, spacing and
    /// quoted names aside, which a migration's `ALTER TABLE` changes.
    fn shape(connection: &Connection) -> Vec<String> {
        let mut shape = Vec::new();
        let mut definitions = connection
            .prepare("SELECT type, name, sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name")
            .unwrap();
        let definitions: Vec<String> = definitions
            .query_map([], |row| {
                let sql: String = row.get(2)?;
                Ok(format!("{} {} {}", row.get::<_, String>(0)?, row.get::<_, String>(1)?, definition(&sql)))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        shape.extend(definitions);
        let mut tables = connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .unwrap();
        let names: Vec<String> = tables
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for table in names {
            for pragma in ["table_xinfo", "index_list", "foreign_key_list"] {
                let mut rows = connection
                    .prepare(&format!("SELECT * FROM pragma_{pragma}(?1)"))
                    .unwrap();
                let width = rows.column_count();
                let mut found: Vec<String> = rows
                    .query_map([&table], |row| {
                        let values: Vec<String> = (0..width)
                            .map(|column| format!("{:?}", row.get_ref(column).unwrap()))
                            .collect();
                        Ok(format!("{table} {pragma} {}", values.join(" ")))
                    })
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                found.sort();
                shape.extend(found);
            }
        }
        let mut indexes = connection
            .prepare("SELECT name, tbl_name FROM sqlite_schema WHERE type = 'index' AND sql IS NOT NULL ORDER BY name")
            .unwrap();
        let indexes: Vec<(String, String)> = indexes
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (index, table) in indexes {
            let mut columns = connection
                .prepare("SELECT seqno, name FROM pragma_index_info(?1) ORDER BY seqno")
                .unwrap();
            // An expression has no column name; the index's definition
            // above holds it.
            let columns: Vec<String> = columns
                .query_map([&index], |row| {
                    let name: Option<String> = row.get(1)?;
                    Ok(name.unwrap_or_else(|| "(expression)".to_owned()))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            shape.push(format!("{table} index {index} {}", columns.join(",")));
        }
        shape
    }

    /// `sql` without its comments, quotes and spacing.
    fn definition(sql: &str) -> String {
        let code: Vec<&str> = sql
            .lines()
            .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
            .collect();
        code.join(" ")
            .replace('"', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(" ,", ",")
    }

    /// A database file that holds nothing yet.
    fn empty() -> (tempfile::TempDir, std::path::PathBuf, Connection) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("database.sqlite");
        let connection = Connection::open(&path).unwrap();
        (directory, path, connection)
    }

    /// A database as a release of the schema `sql` made it.
    fn database(sql: &str) -> (tempfile::TempDir, std::path::PathBuf, Connection) {
        let (directory, path, connection) = empty();
        connection.execute_batch(sql).unwrap();
        connection
            .pragma_update(None, "user_version", version_of(sql))
            .unwrap();
        (directory, path, connection)
    }

    const SHIPPED: &str = "CREATE TABLE notes (id TEXT PRIMARY KEY) STRICT;";
    const CURRENT: &str =
        "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '') STRICT;";

    /// A database of a shipped schema becomes one of the current schema
    /// with its rows; one whose migration fails stays as it was; one of a
    /// schema the history does not hold is refused.
    #[test]
    fn a_shipped_schema_migrates_a_failed_migration_changes_nothing_and_another_is_refused() {
        let schema = Schema {
            sql: CURRENT,
            history: &[Shipped {
                sql: SHIPPED,
                migration: Migration::Sql("ALTER TABLE notes ADD COLUMN title TEXT NOT NULL DEFAULT '';"),
            }],
        };
        let (_directory, path, mut connection) = database(SHIPPED);
        connection.execute("INSERT INTO notes (id) VALUES ('a')", []).unwrap();
        assert_eq!(schema.current(version_of(SHIPPED)), Some(false));
        schema.apply(&mut connection, &path).unwrap();
        let version: i32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, version_of(CURRENT));
        let title: String = connection
            .query_row("SELECT title FROM notes WHERE id = 'a'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "");
        let (_fresh_directory, fresh_path, mut fresh) = empty();
        schema.apply(&mut fresh, &fresh_path).unwrap();
        assert_eq!(shape(&connection), shape(&fresh));

        let failing = Schema {
            sql: CURRENT,
            history: &[Shipped {
                sql: SHIPPED,
                migration: Migration::Code(|transaction| {
                    transaction.execute_batch("ALTER TABLE notes ADD COLUMN title TEXT NOT NULL DEFAULT '';")?;
                    transaction.execute_batch("INSERT INTO missing VALUES (1);")
                }),
            }],
        };
        let (_failing_directory, failing_path, mut failed) = database(SHIPPED);
        let refused = failing.apply(&mut failed, &failing_path);
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
        let version: i32 = failed
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, version_of(SHIPPED));
        let columns: i64 = failed
            .query_row("SELECT count(*) FROM pragma_table_info('notes')", [], |row| row.get(0))
            .unwrap();
        assert_eq!(columns, 1);

        let (_other_directory, other_path, mut other) = database("CREATE TABLE other (id TEXT) STRICT;");
        let refused = schema.apply(&mut other, &other_path);
        assert!(matches!(refused, Err(StorageError::OtherSchema { .. })), "{refused:?}");
        assert_eq!(schema.current(version_of("CREATE TABLE other (id TEXT) STRICT;")), None);
    }

    /// Every schema a published release shipped migrates to the current
    /// one: the same tables, columns, indexes, constraints and foreign keys
    /// as a new database.
    #[test]
    fn each_shipped_schema_migrates_to_the_current_one() {
        for schema in [CONTROL, CONVERSATION] {
            let (_fresh_directory, fresh_path, mut fresh) = empty();
            schema.apply(&mut fresh, &fresh_path).unwrap();
            for shipped in schema.history {
                let (_directory, path, mut migrated) = database(shipped.sql);
                schema.apply(&mut migrated, &path).unwrap();
                assert_eq!(shape(&migrated), shape(&fresh));
            }
        }
    }

    /// 0.1.11's rows of a conversation keep their values through the
    /// rebuilt tables, and the attachment sequence counts in it; an output
    /// 0.1.11 removed, which has no state now, stops the migration and
    /// leaves the database as it was.
    #[test]
    fn a_conversation_of_0_1_11_keeps_its_rows_and_an_output_it_removed_stops_the_migration() {
        use demi_host_interface::{MediumKept, Missing, StoredMedium};
        use demi_shared_types::{BlobRef, CommandId, Sequence, Timestamp};

        use crate::command_outputs::{self, CommandOutput, OutputRow};
        use crate::sequences;

        let shipped = CONVERSATION.history[0].sql;
        let blob = BlobRef::try_from("a".repeat(64)).unwrap();
        let media = vec![
            StoredMedium {
                number: 1,
                media_type: "image/png".to_owned(),
                size: 12,
                kept: MediumKept::Stored { blob: blob.clone() },
            },
            StoredMedium {
                number: 2,
                media_type: "video/mp4".to_owned(),
                size: 40,
                kept: MediumKept::Missing {
                    reason: "lost with the Host's connection".to_owned(),
                },
            },
        ];
        let (_directory, path, mut connection) = database(shipped);
        connection
            .execute_batch("INSERT INTO sequences (name, next) VALUES ('command', 7), ('tab', 3);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO command_outputs
                   (command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored, removed_at)
                 VALUES ('c1', 1000, ?1, 25, 'lost with the Host''s connection', ?2, NULL, NULL),
                        ('c2', 2000, NULL, NULL, NULL, NULL, 'the object store refused the write', NULL)",
                rusqlite::params![blob.as_str(), serde_json::to_string(&media).unwrap()],
            )
            .unwrap();

        CONVERSATION.apply(&mut connection, &path).unwrap();
        let stored = command_outputs::read(&connection, &CommandId::try_from("c1").unwrap()).unwrap();
        assert_eq!(
            stored,
            Some(CommandOutput {
                command: CommandId::try_from("c1").unwrap(),
                ended: Timestamp::from_millisecond(1000).unwrap(),
                output: OutputRow::Stored {
                    blob,
                    missing: Some(Missing {
                        bytes: 25,
                        reason: "lost with the Host's connection".to_owned(),
                    }),
                    media,
                },
            })
        );
        let not_stored = command_outputs::read(&connection, &CommandId::try_from("c2").unwrap()).unwrap();
        assert_eq!(
            not_stored.map(|row| row.output),
            Some(OutputRow::NotStored("the object store refused the write".to_owned()))
        );
        assert_eq!(
            sequences::all(&connection).unwrap(),
            vec![(Sequence::Command, 7), (Sequence::Tab, 3)]
        );
        assert_eq!(sequences::next(&connection, Sequence::Attachment).unwrap(), 1);

        let (_removed_directory, removed_path, mut removed) = database(shipped);
        removed
            .execute_batch(
                "INSERT INTO command_outputs (command_id, ended_at, removed_at) VALUES ('c3', 1000, 2000);",
            )
            .unwrap();
        let refused = CONVERSATION.apply(&mut removed, &removed_path);
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
        let version: i32 = removed
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, version_of(shipped));
        let removed_at: i64 = removed
            .query_row("SELECT removed_at FROM command_outputs WHERE command_id = 'c3'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(removed_at, 2000);
    }
}
