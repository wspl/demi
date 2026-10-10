//! The schemas of the control database and of each conversation's database
//! (`storage.md` § Schemas and migrations): one SQL text each, applied to a
//! new database in a transaction, whose digest is the version a database
//! records, and the history of the schemas published releases shipped
//! before it, each with the migration to the next. Times are integer
//! milliseconds since the Unix epoch; a closed set is text a CHECK limits;
//! JSON columns are text their reader decodes and validates; sealed values
//! are BLOBs.

use std::{collections::HashMap, path::Path};

use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use demi_agent_store::CheckpointState;
use demi_shared_types::ReportEvent;

use super::StorageError;
use super::columns::{decode, json, to_json};

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
    Code(fn(&Transaction<'_>) -> rusqlite::Result<()>),
}

/// The control database's. Its history holds the schema of each published
/// release before the one that ships the current schema; 0.1.21 shipped the
/// last one in it.
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
        Shipped {
            sql: include_str!("schema/control-0.1.17.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_17),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.18.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_18),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.20.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_20),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.21.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_21),
        },
    ],
};

/// Each conversation's database's. Its history holds the schema of each
/// published release before the one that ships the current schema; 0.1.21
/// shipped the last one in it.
pub(crate) const CONVERSATION: Schema = Schema {
    sql: CONVERSATION_V1,
    history: &[
        Shipped {
            sql: include_str!("schema/conversation-0.1.11.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_0_1_11),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.20.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_0_1_20),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.21.sql"),
            migration: Migration::Code(from_0_1_21),
        },
    ],
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

/// From 0.1.16's control schema: a device keeps the runner release its
/// runner reports, which a device migrated from 0.1.16 learns at its
/// runner's next hello; and a conversation's deletion is recorded as
/// pending until its last step (`storage.md` § Deleting a conversation).
const CONTROL_FROM_0_1_16: &str = "
ALTER TABLE devices ADD COLUMN runner_version TEXT;
CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  user_id    TEXT NOT NULL REFERENCES users (id),
  deleted_at INTEGER NOT NULL
) STRICT;
";

/// From 0.1.17's control schema: a device records whether the backend's
/// shutdown ended its last connection, which no device migrated from 0.1.17
/// did.
const CONTROL_FROM_0_1_17: &str = "
ALTER TABLE devices ADD COLUMN ended_by_shutdown INTEGER NOT NULL DEFAULT 0 CHECK (ended_by_shutdown IN (0, 1));
";

/// From 0.1.18's control schema: the deployment keeps its namespace at the
/// preview domain, which a deployment migrated from 0.1.18 registers at its
/// next start.
const CONTROL_FROM_0_1_18: &str = "
CREATE TABLE preview_namespace (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  namespace  TEXT NOT NULL,
  secret     BLOB NOT NULL,
  expires_at INTEGER NOT NULL,
  origins    TEXT NOT NULL
) STRICT;
";

/// From 0.1.20's control schema: a device keeps how pages reach it, which
/// is Automatic for every device migrated from 0.1.20, as for a new one
/// (`direct-channel.md` § Choosing the path).
const CONTROL_FROM_0_1_20: &str = "
ALTER TABLE devices ADD COLUMN route TEXT NOT NULL DEFAULT 'automatic' CHECK (route IN ('automatic', 'direct', 'server'));
";

/// From 0.1.21's control schema: a conversation keeps the move an agent
/// asked for and an attached host the detach it waits with, none for every
/// conversation 0.1.21 left; a permission request names its categories, the
/// one category of each request 0.1.21 stored. SQLite cannot drop a column
/// an index names, so the requests' table is made anew, as for 0.1.11's
/// conversations. The web preview is gone, and the deployment's namespace
/// at the preview domain with it. Users keep personal instructions, which
/// no user migrated from 0.1.21 has. Yield wakeups are gone, and the index
/// of when each conversation's earliest one is due with them. Running jobs
/// are recorded by device, none of which 0.1.21 kept, since its jobs ended
/// with their connections.
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

/// From 0.1.20's conversation schema: an output's record keeps how its
/// command ended, which no row 0.1.20 wrote says, so theirs stays null. The
/// table is made anew, as for 0.1.11, so the column takes its place among
/// the others.
const CONVERSATION_FROM_0_1_20: &str = "
CREATE TABLE command_outputs_next (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  ending         TEXT,
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
";

/// From 0.1.21's conversation schema: commands no longer run in shells, the
/// model has one tool, `shell`, and command reports replace yield wakeups
/// (`runtime.md` § Command reports). Block format 2 also lets a block keep
/// the entries of a vendor's own record of the session (`claude-code.md`
/// § The session a process resumes), which no block 0.1.21 stored has, so
/// leaving the field out writes them as format 2. Commands outlive their
/// connections: a running one is recorded, none of which 0.1.21 kept, and a
/// lost one says why (command ending format 2), which for 0.1.21's, lost
/// with their connections, is that.
fn from_0_1_21(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
DELETE FROM sequences WHERE name = 'shell';
CREATE TABLE sequences_next (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;
INSERT INTO sequences_next (name, next) SELECT name, next FROM sequences;
DROP TABLE sequences;
ALTER TABLE sequences_next RENAME TO sequences;
ALTER TABLE nodes DROP COLUMN wakeup_at;
CREATE TABLE running_commands (
  command_id  TEXT PRIMARY KEY,
  node_id     TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  device_id   TEXT NOT NULL,
  job_id      TEXT NOT NULL,
  tool_use_id TEXT NOT NULL,
  started_at  INTEGER NOT NULL
) STRICT;
CREATE INDEX running_commands_of_node ON running_commands (node_id);
",
    )?;
    transaction.execute(
        "UPDATE command_outputs SET ending = json_set(ending, '$.reason', ?1)
         WHERE json_extract(ending, '$.kind') = 'lost'",
        [LOST_IN_0_1_21],
    )?;
    blocks_of_one_tool(transaction)?;
    states_without_wakeups(transaction)
}

/// Turns a stored error into the migration's failure, which leaves the
/// database as 0.1.21 left it.
fn corrupt(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
}

/// From 0.1.21's checkpoint states (checkpoint state format 2): the yield
/// wakeups a node saved go, none of them to fire, and the state keeps the
/// command reports and the commands' intervals, none for a node 0.1.21
/// saved. Each state is decoded and checked as the format it becomes before
/// it is written back.
fn states_without_wakeups(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let states = transaction
        .prepare("SELECT id, state FROM nodes")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE nodes SET state = ?2 WHERE id = ?1")?;
    for (id, state) in states {
        let mut fields: serde_json::Map<String, serde_json::Value> =
            decode("nodes", "state", serde_json::from_str(&state)).map_err(corrupt)?;
        fields.remove("wakeups");
        fields.insert("reports".to_owned(), serde_json::json!([]));
        fields.insert("intervals".to_owned(), serde_json::json!([]));
        let state: CheckpointState = json("nodes", "state", &to_json(&fields)).map_err(corrupt)?;
        update.execute(rusqlite::params![id, to_json(&state)])?;
    }
    Ok(())
}

/// From 0.1.21's transcript blocks (transcript block format 2): a stored
/// call of `shell_exec` is the `shell` call it became, with its input, and
/// its repeat guard's view is `repeated_shell`; a call of `yield` keeps its
/// name, which replays as text (`runtime.md` § Replay), and loses its
/// `yield_wakeup` view, which no tool has any more; a stored shell view no
/// longer names a shell; and a fired wakeup becomes what it stood for
/// (`runtime.md` § Rendering boundary): one a command's end fired, that
/// command's report, titled by the `description` of the call that started
/// the command; one its time fired, a `resume` block. Each block changed is
/// decoded and written again through the block's own encoding; a block that
/// does not decode stops the migration.
fn blocks_of_one_tool(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let titles = command_titles_of_0_1_21(transaction)?;
    let mut blocks = Vec::new();
    {
        let mut statement = transaction.prepare("SELECT node_id, idx, block FROM blocks")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let text: String = row.get(2)?;
            let mut block: serde_json::Value = serde_json::from_str(&text).map_err(corrupt)?;
            if migrate_block(&mut block, &titles) {
                let block: demi_shared_types::Block =
                    demi_shared_types::decode_value(block).map_err(corrupt)?;
                let node: String = row.get(0)?;
                let index: i64 = row.get(1)?;
                blocks.push((node, index, super::columns::to_json(&block)));
            }
        }
    }
    let mut update = transaction.prepare("UPDATE blocks SET block = ?3 WHERE node_id = ?1 AND idx = ?2")?;
    for (node, index, block) in blocks {
        update.execute(rusqlite::params![node, index, block])?;
    }
    Ok(())
}

/// The title of each command a call of 0.1.21's started, by its number: the
/// call's `description`, as the stored view of a `shell_exec` call names its
/// command. A `shell_status` look's view names the command it looked at,
/// which it did not start.
fn command_titles_of_0_1_21(transaction: &Transaction<'_>) -> rusqlite::Result<HashMap<String, String>> {
    use serde_json::Value;

    let mut titles = HashMap::new();
    let mut statement = transaction.prepare("SELECT block FROM blocks ORDER BY node_id, idx")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let text: String = row.get(0)?;
        let block: Value = serde_json::from_str(&text).map_err(corrupt)?;
        if block["type"] != "tool_call" || block["toolName"] != "shell_exec" {
            continue;
        }
        let Some(command) = block["view"]["commandId"].as_str() else {
            continue;
        };
        let input: Value = block["input"]
            .as_str()
            .and_then(|input| serde_json::from_str(input).ok())
            .unwrap_or(Value::Null);
        let title = input["description"].as_str().unwrap_or_default();
        titles.entry(command.to_owned()).or_insert_with(|| title.to_owned());
    }
    Ok(titles)
}

/// Changes one of 0.1.21's blocks into what it became; false when it stays
/// as it is. `titles` names the commands 0.1.21's calls started.
fn migrate_block(block: &mut serde_json::Value, titles: &HashMap<String, String>) -> bool {
    use serde_json::Value;

    let Some(fields) = block.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    match fields.get("type").and_then(Value::as_str) {
        Some("tool_call") => {
            if fields.get("toolName").and_then(Value::as_str) == Some("shell_exec") {
                fields.insert("toolName".to_owned(), Value::from("shell"));
                changed = true;
            }
            let kind = fields
                .get("view")
                .and_then(|view| view.get("kind"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            match kind.as_deref() {
                Some("shell") => {
                    if let Some(view) = fields.get_mut("view").and_then(Value::as_object_mut) {
                        changed |= view.remove("shellId").is_some();
                    }
                }
                Some("repeated_shell_exec") => {
                    fields["view"]["kind"] = Value::from("repeated_shell");
                    changed = true;
                }
                Some("yield_wakeup") => {
                    fields.insert("view".to_owned(), Value::Null);
                    changed = true;
                }
                _ => {}
            }
        }
        Some("wakeup") => {
            match fields.remove("command") {
                Some(command) => {
                    let report = report_of_0_1_21(&command, titles);
                    fields.insert("reports".to_owned(), Value::Array(vec![report]));
                }
                None => {
                    fields.insert("type".to_owned(), Value::from("resume"));
                    fields.remove("placement");
                }
            }
            changed = true;
        }
        _ => {}
    }
    changed
}

/// Why every command 0.1.21 recorded lost was lost.
const LOST_IN_0_1_21: &str = "the connection to its Host was lost";

/// The report a wakeup 0.1.21 fired for `command`'s end stood for: the
/// command, its call's title, and how it ended. It carries no output: the
/// model read none with it then.
fn report_of_0_1_21(command: &serde_json::Value, titles: &HashMap<String, String>) -> serde_json::Value {
    let id = command["commandId"].as_str().unwrap_or_default();
    let end = &command["end"];
    let event = match end["kind"].as_str() {
        Some("exited") => ReportEvent::Ended {
            exit_code: end["exitCode"].as_i64().and_then(|code| i32::try_from(code).ok()),
        },
        Some("stopped") => ReportEvent::Stopped { by: None },
        // 0.1.21 lost a command only with its Host's connection.
        Some("lost") => ReportEvent::Lost {
            reason: LOST_IN_0_1_21.to_owned(),
        },
        _ => ReportEvent::Ended { exit_code: None },
    };
    serde_json::json!({
        "commandId": id,
        "title": titles.get(id).cloned().unwrap_or_default(),
        "event": event,
        "output": "",
    })
}

/// A kind of database, as a server's upgrade asks about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Control,
    Conversation,
}

impl DatabaseKind {
    fn schema(self) -> &'static Schema {
        match self {
            Self::Control => &CONTROL,
            Self::Conversation => &CONVERSATION,
        }
    }
}

/// What this build does with a database as it opens it (`upgrades.md`
/// § Prepare).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaFit {
    /// It records this build's schema and opens as it is.
    Current,
    /// It holds nothing yet or records a schema in the history: opening
    /// gives it this build's schema.
    Migrates,
    /// It records another schema, as a newer release or a development build
    /// of another unpublished schema made it: opening refuses it.
    Other,
}

/// What this build would do with the database at `path` as it opens it. The
/// database is read, never written.
pub fn schema_fit(path: &Path, kind: DatabaseKind) -> Result<SchemaFit, StorageError> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let recorded: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    // A database that records no version and holds no table is new, and
    // `apply` gives it the schema.
    if recorded == 0 && tables(&connection)? == 0 {
        return Ok(SchemaFit::Migrates);
    }
    Ok(match kind.schema().current(recorded) {
        Some(true) => SchemaFit::Current,
        Some(false) => SchemaFit::Migrates,
        None => SchemaFit::Other,
    })
}

/// How many tables the database of `connection` holds.
fn tables(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
        [],
        |row| row.get(0),
    )
}

/// The versions a database of `kind` records, for the tests of what reads
/// only a database's version, as an upgrade's preparation does.
#[cfg(feature = "testing")]
pub mod testing {
    use super::{DatabaseKind, version_of};

    /// The version of this build's schema.
    pub fn current_version(kind: DatabaseKind) -> i32 {
        kind.schema().version()
    }

    /// The version of the newest schema a published release shipped before
    /// this build's.
    pub fn shipped_version(kind: DatabaseKind) -> i32 {
        let shipped = kind.schema().history.last().expect("a schema has a history");
        version_of(shipped.sql)
    }
}

/// The version a database of the schema `sql` records in `user_version`:
/// the first 31 bits of its SHA-256, never 0, which SQLite gives a database
/// that records none. Any edit of the SQL is another version.
pub(crate) fn version_of(sql: &str) -> i32 {
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
        if recorded == 0 && tables(&transaction)? == 0 {
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

-- Each user's personal instructions (`instructions.md` § Personal
-- instructions): a user with no row has none.
CREATE TABLE user_instructions (
  user_id TEXT PRIMARY KEY REFERENCES users (id),
  text    TEXT NOT NULL
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
  -- The operating system its runner last reported, JSON, and the runner
  -- release it reported with it; none before its runner first connected.
  os           TEXT,
  runner_version TEXT,
  -- Whether the backend's last shutdown ended its runner's connection; the
  -- next start reads it and clears it.
  ended_by_shutdown INTEGER NOT NULL DEFAULT 0 CHECK (ended_by_shutdown IN (0, 1)),
  -- How pages reach it: Automatic, Prefer Direct or Server Only, which the
  -- user picks on the device's page.
  route        TEXT NOT NULL DEFAULT 'automatic' CHECK (route IN ('automatic', 'direct', 'server'))
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
  -- The revision of its attached hosts, raised by each change of them; 0
  -- before the first.
  hosts_revision      INTEGER NOT NULL DEFAULT 0 CHECK (hosts_revision >= 0),
  -- The pending move an agent asked for (sessions-and-targets.md § Switch
  -- the primary target): its id and its target, typed as the target is; all
  -- null while none waits.
  pending_id           TEXT,
  pending_kind         TEXT CHECK (pending_kind IN ('cloud', 'device', 'workspace')),
  pending_device_id    TEXT,
  pending_path         TEXT,
  pending_workspace_id TEXT CHECK (
    (pending_kind IS NULL
      AND pending_id IS NULL AND pending_device_id IS NULL AND pending_path IS NULL
      AND pending_workspace_id IS NULL)
    OR (pending_kind = 'cloud' AND pending_id IS NOT NULL
      AND pending_device_id IS NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'device' AND pending_id IS NOT NULL
      AND pending_device_id IS NOT NULL AND pending_path IS NOT NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'workspace' AND pending_id IS NOT NULL
      AND pending_workspace_id IS NOT NULL AND pending_device_id IS NULL AND pending_path IS NULL)
  ),
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

-- Each job of a command that runs, with its device, whose runner's hello
-- names the job, and its conversation, whose tree takes the command up
-- again (`storage.md` § Command outputs). Written before the job starts and
-- deleted with the command's end.
CREATE TABLE running_jobs (
  job_id          TEXT PRIMARY KEY,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;
CREATE INDEX running_jobs_of_device ON running_jobs (device_id);

CREATE TABLE conversation_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id),
  device_id       TEXT NOT NULL REFERENCES devices (id),
  name            TEXT NOT NULL,
  -- The directory its commands start in, fixed when it was attached; null
  -- for a device attached by name, whose commands start in its home.
  cwd             TEXT,
  attached_at     INTEGER NOT NULL,
  -- An agent's detach, which waits for the tree to be idle.
  detaching       INTEGER NOT NULL DEFAULT 0 CHECK (detaching IN (0, 1)),
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
-- without the grants of its categories, a JSON array of their ids sorted so
-- that the same categories compare equal, undecided until the user decides and
-- then kept until the decision's message is in the agent's checkpoint; and
-- each category the user allowed for a conversation. The agent is the root
-- when its number and description are null.
CREATE TABLE permission_requests (
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
  -- nodes.state: checkpoint state format 2
  state            TEXT NOT NULL,
  block_count      INTEGER NOT NULL CHECK (block_count >= 0),
  output_revision  INTEGER NOT NULL CHECK (output_revision >= 0),
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
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

-- blocks.block: transcript block format 2.
CREATE TABLE blocks (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx     INTEGER NOT NULL CHECK (idx >= 0),
  -- blocks.block: transcript block format 2
  block   TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
) STRICT;

-- Each ended command's whole output: stored as a blob, with the bytes at
-- its end the backend does not have and why; or not stored, and why.
CREATE TABLE command_outputs (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  -- How it ended, a JSON object: its exit code, stopped, or with its Host's
  -- connection; null for a row a release before 0.1.21 wrote.
  -- command_outputs.ending: command ending format 2
  ending         TEXT,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  -- The command's media, a JSON array, beside a stored output.
  -- command_outputs.media: command media format 1
  media          TEXT,
  not_stored     TEXT,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL),
  CHECK ((media IS NULL) = (blob IS NULL))
) STRICT;

-- Each command that runs, until its end writes its command_outputs row in
-- the same transaction: the node that ran it, its device, its job's id on
-- that device, the call that started it, and when it started.
CREATE TABLE running_commands (
  command_id  TEXT PRIMARY KEY,
  node_id     TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  device_id   TEXT NOT NULL,
  job_id      TEXT NOT NULL,
  tool_use_id TEXT NOT NULL,
  started_at  INTEGER NOT NULL
) STRICT;
CREATE INDEX running_commands_of_node ON running_commands (node_id);

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
        use demi_shared_types::{BlobRef, CommandEnd, CommandId, Sequence, Timestamp};

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
                // Its release kept no end, and the migrations invent none.
                end: CommandEnd::Unrecorded,
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

    /// A node's checkpoint state as 0.1.21 saved it, with the yield
    /// wakeups `wakeups`.
    fn state_of_0_1_21(wakeups: serde_json::Value) -> String {
        serde_json::json!({
            "phase": "idle",
            "queue": [],
            "agentInputs": [],
            "wakeups": wakeups,
            "cwd": "/w",
            "model": demi_agent_store::testing::test_model(),
            "edits": []
        })
        .to_string()
    }

    /// 0.1.21's checkpoint states lose their yield wakeups and hold no
    /// command report and no command's interval; a state that is not one of
    /// 0.1.21's stops the migration.
    #[test]
    fn a_conversation_of_0_1_21_loses_its_wakeups() {
        use crate::columns::json;

        let shipped = CONVERSATION.history[2].sql;
        let insert = "INSERT INTO nodes (id, number, parent_id, description, round, started_at, can_spawn,
                        delivered, state, block_count, output_revision, wakeup_at)
                      VALUES (?1, ?2, ?3, '', 1, 0, 1, 0, ?4, 0, 0, ?5)";
        let wakeup = serde_json::json!([{"id": "w1", "durationMs": 60000, "dueAt": null}]);
        let (_directory, path, mut connection) = database(shipped);
        connection
            .execute(
                insert,
                rusqlite::params!["root", 0, None::<String>, state_of_0_1_21(serde_json::json!([])), None::<i64>],
            )
            .unwrap();
        connection
            .execute(insert, rusqlite::params!["waiting", 1, "root", state_of_0_1_21(wakeup), 0])
            .unwrap();

        CONVERSATION.apply(&mut connection, &path).unwrap();
        for id in ["root", "waiting"] {
            let state: String = connection
                .query_row("SELECT state FROM nodes WHERE id = ?1", [id], |row| row.get(0))
                .unwrap();
            let state = json::<CheckpointState>("nodes", "state", &state).unwrap();
            assert!(state.reports.is_empty() && state.intervals.is_empty(), "{id}");
        }

        let (_corrupt_directory, corrupt_path, mut corrupt) = database(shipped);
        corrupt
            .execute(
                insert,
                rusqlite::params!["root", 0, None::<String>, "{\"phase\":\"idle\"}", None::<i64>],
            )
            .unwrap();
        let refused = CONVERSATION.apply(&mut corrupt, &corrupt_path);
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
    }

    /// 0.1.21's shell sequence goes with its row; a stored `shell_exec`
    /// call is a `shell` call with its input, its shell view naming no
    /// shell and its repeat guard's view `repeated_shell`; a `yield` and a
    /// `shell_status` call keep their names, and the `yield` loses its
    /// view; a wakeup a command's end fired is that command's report, titled
    /// by the call that started it, and one its time fired a resume. Every
    /// other number keeps its next value.
    #[test]
    fn a_conversation_of_0_1_21_becomes_one_of_the_shell_tool() {
        use demi_shared_types::{Block, CommandId, CommandReport, ReportEvent, Sequence, ToolView};

        use crate::sequences;

        let shipped = CONVERSATION
            .history
            .last()
            .expect("a schema has a history")
            .sql;
        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-sonnet-4-5",
                "name": "Claude Sonnet 4.5",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "disabled"}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "disabled"},
            "serviceTierId": null
        });
        let call = |id: &str, tool: &str, input: &str, view: serde_json::Value| {
            serde_json::json!({
                "type": "tool_call",
                "id": id,
                "createdAt": "2026-09-21T14:13:20.000Z",
                "model": model,
                "toolUseId": format!("toolu_{id}"),
                "toolName": tool,
                "input": input,
                "status": "completed",
                "output": [{"type": "text", "text": "done"}],
                "view": view
            })
        };
        let shell_view = serde_json::json!({
            "kind": "shell",
            "status": "running",
            "commandId": "17",
            "runningMs": 60000,
            "idleMs": 2000,
            "chunks": [],
            "viewTruncated": false,
            "shellId": "3"
        });
        let exec_input = r#"{"script":"npm test","description":"Run the tests","timeoutMs":600000}"#;
        let blocks = [
            call("c1", "shell_exec", exec_input, shell_view),
            call(
                "c2",
                "shell_exec",
                exec_input,
                serde_json::json!({"kind": "repeated_shell_exec", "script": "npm test", "count": 7}),
            ),
            call(
                "c3",
                "yield",
                r#"{"durationMs":600000,"commandIds":[17]}"#,
                serde_json::json!({"kind": "yield_wakeup", "wakeupId": "w1", "durationMs": 600000, "commandIds": ["17"]}),
            ),
            call("c4", "shell_status", r#"{"commandId":17}"#, serde_json::Value::Null),
            serde_json::json!({
                "type": "wakeup",
                "id": "w1",
                "turnId": "t1",
                "createdAt": "2026-09-21T14:13:20.000Z",
                "model": model,
                "placement": "new_turn",
                "command": {"commandId": "17", "end": {"kind": "exited", "exitCode": 1}}
            }),
            serde_json::json!({
                "type": "wakeup",
                "id": "w2",
                "turnId": "t2",
                "createdAt": "2026-09-21T14:13:20.000Z",
                "model": model,
                "placement": "steer"
            }),
        ];
        let (_directory, path, mut connection) = database(shipped);
        connection
            .execute_batch(
                "INSERT INTO sequences (name, next) VALUES ('command', 18), ('shell', 4), ('agent', 2);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, 6, 0)",
                [state_of_0_1_21(serde_json::json!([]))],
            )
            .unwrap();
        for (index, block) in blocks.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                )
                .unwrap();
        }

        CONVERSATION.apply(&mut connection, &path).unwrap();
        let stored: Vec<Block> = connection
            .prepare("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|text| demi_shared_types::decode::<Block>(&text.unwrap()).unwrap())
            .collect();
        let calls: Vec<(&str, &str, Option<&ToolView>)> = stored
            .iter()
            .filter_map(|block| match block {
                Block::ToolCall(call) => {
                    Some((call.tool_name.as_str(), call.input.as_str(), call.view.as_ref()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(calls[0].0, "shell");
        assert_eq!(calls[0].1, exec_input);
        assert!(matches!(calls[0].2, Some(ToolView::Shell(view)) if view.command_id.as_str() == "17"));
        assert_eq!(
            calls[1].2,
            Some(&ToolView::RepeatedShell {
                script: "npm test".into(),
                count: 7
            })
        );
        assert_eq!((calls[2].0, calls[2].2), ("yield", None));
        assert_eq!((calls[3].0, calls[3].2), ("shell_status", None));
        // The wakeup command 17's end fired is its report, titled by the
        // call that started it; the one its time fired is a resume.
        match &stored[4] {
            Block::Wakeup(wakeup) => assert_eq!(
                wakeup.reports,
                [CommandReport {
                    command_id: CommandId::try_from("17").unwrap(),
                    title: "Run the tests".to_owned(),
                    event: ReportEvent::Ended { exit_code: Some(1) },
                    output: String::new(),
                }]
            ),
            other => panic!("{other:?}"),
        }
        assert!(matches!(&stored[5], Block::Resume(resume) if resume.turn_id.as_str() == "t2"));
        assert_eq!(
            sequences::all(&connection).unwrap(),
            vec![(Sequence::Agent, 2), (Sequence::Command, 18)]
        );
        assert!(
            connection
                .execute("INSERT INTO sequences (name, next) VALUES ('shell', 1)", [])
                .is_err()
        );
    }
}
