//! From the control schema that held every user's records to the one that
//! holds only what the server needs above its users' cores.
//!
//! Each user's records leave for `users/<id>/core.sqlite`, with no column
//! naming the user. The user's conversation databases follow to
//! `users/<id>/conversations/` and the search index to
//! `users/<id>/search.sqlite`. The blobs stay in the object store, in the
//! namespace each user has.
//!
//! A core's database is written under another name and takes its own name when
//! it is whole. The control database commits last, so a start that stops
//! anywhere runs the step again: it writes each core's database anew and moves
//! the files it has not moved.

use std::path::Path;

use rusqlite::types::Value;
use rusqlite::{Connection, Transaction, params_from_iter};

use super::super::{ControlSteps, Migration, corrupt};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_65_one_core_per_user);

/// The schema of a core's database as this step writes it.
const CORE_SCHEMA: &str = include_str!("core_at_one_core_per_user.sql");

/// A conversation of the user, in a condition on a table's `conversation_id`.
const OF_CONVERSATIONS: &str = "conversation_id IN (SELECT id FROM conversations WHERE user_id = ?1)";
/// A provider entry of the user, in a condition on a table's `provider_id`.
const OF_PROVIDERS: &str = "provider_id IN (SELECT id FROM providers WHERE owner_user_id = ?1)";
/// A device of the user, in a condition on a table's `device_id`.
const OF_DEVICES: &str = "device_id IN (SELECT id FROM devices WHERE user_id = ?1)";
/// A schedule of the user, in a condition on a table's `schedule_id`.
const OF_SCHEDULES: &str =
    "schedule_id IN (SELECT id FROM schedules WHERE conversation_id IN (SELECT id FROM conversations WHERE user_id = ?1))";
/// The row of the user, in a condition on a table's `user_id`.
const OF_USER: &str = "user_id = ?1";

/// The tables a core's database takes, each before those that name it, with
/// the condition that picks the rows of the user `?1`.
const TABLES: &[(&str, &str)] = &[
    ("user_preferences", OF_USER),
    ("user_instructions", OF_USER),
    ("user_subagents", OF_USER),
    ("subagent_profiles", OF_USER),
    ("devices", OF_USER),
    ("workspaces", OF_USER),
    ("projects", OF_USER),
    ("directory_worktrees", OF_DEVICES),
    ("port_forwards", OF_USER),
    ("host_workflows", OF_DEVICES),
    ("user_plugins", OF_USER),
    ("user_plugin_features", OF_USER),
    ("plugin_values", OF_USER),
    ("plugin_directories", OF_USER),
    ("plugin_secrets", OF_USER),
    // A client an issuer gave the whole server: each core keeps using it.
    ("plugin_oauth_clients", "?1 = ?1"),
    ("user_credentials", OF_USER),
    ("conversations", OF_USER),
    ("schedules", OF_CONVERSATIONS),
    ("schedule_runs", OF_SCHEDULES),
    ("schedule_calls", OF_SCHEDULES),
    ("running_jobs", OF_CONVERSATIONS),
    ("conversation_hosts", OF_CONVERSATIONS),
    ("conversation_panels", OF_CONVERSATIONS),
    ("plugin_reports", OF_CONVERSATIONS),
    ("conversation_drafts", OF_CONVERSATIONS),
    ("conversation_suggested_replies", OF_CONVERSATIONS),
    (
        "saved_messages",
        "conversation_id IN (SELECT id FROM conversations WHERE user_id = ?1)
         OR project_id IN (SELECT id FROM projects WHERE user_id = ?1)",
    ),
    ("saved_sends", OF_CONVERSATIONS),
    ("permission_requests", OF_CONVERSATIONS),
    ("permission_grants", OF_CONVERSATIONS),
    ("interrupted_turns", OF_CONVERSATIONS),
    ("idle_compactions", OF_CONVERSATIONS),
    ("conversation_deletions", OF_USER),
    ("conversation_fork_operations", OF_USER),
    ("providers", "owner_user_id = ?1"),
    ("provider_credentials", OF_PROVIDERS),
    ("model_catalogs", OF_PROVIDERS),
    ("retired_models", OF_PROVIDERS),
    ("usage_ledger", OF_USER),
    // A deleted conversation's cost stays with its rows of the ledger.
    (
        "usage_costs",
        "conversation_id IN (SELECT conversation_id FROM usage_ledger WHERE user_id = ?1)",
    ),
    ("media_jobs", OF_CONVERSATIONS),
    ("attachments", OF_USER),
];

fn control_from_0_1_65_one_core_per_user(transaction: &Transaction<'_>, steps: &dyn ControlSteps) -> rusqlite::Result<()> {
    let data = steps.data_directory();
    let users: Vec<String> = transaction
        .prepare("SELECT id FROM users")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for user in &users {
        let directory = data.join("users").join(user);
        write_core(transaction, user, &directory)?;
        move_conversations(transaction, user, data, &directory)?;
        move_database(&data.join("search").join(format!("{user}.sqlite")), &directory.join("search.sqlite"))?;
    }
    // The directories go once every file has moved; a file no record named
    // keeps its directory.
    for left in ["conversations", "search"] {
        remove_empty_directory(&data.join(left))?;
    }
    transaction.execute_batch(CONTROL_RECORDS)?;
    for (table, _) in TABLES.iter().rev() {
        transaction.execute_batch(&format!("DROP TABLE {table};"))?;
    }
    Ok(())
}

/// What stays in the control database in its new form, and what it now keeps
/// of the records that left.
const CONTROL_RECORDS: &str = "
CREATE TABLE device_owners (
  device_id  TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  token_hash TEXT UNIQUE
) STRICT, WITHOUT ROWID;
INSERT INTO device_owners (device_id, user_id, token_hash) SELECT id, user_id, token_hash FROM devices;

CREATE TABLE schedule_owners (
  schedule_id TEXT PRIMARY KEY,
  user_id     TEXT NOT NULL REFERENCES users (id)
) STRICT, WITHOUT ROWID;
INSERT INTO schedule_owners (schedule_id, user_id)
  SELECT s.id, c.user_id FROM schedules s JOIN conversations c ON c.id = s.conversation_id;

CREATE TABLE shares_new (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users (id),
  conversation_id TEXT NOT NULL COLLATE NOCASE,
  kind            TEXT NOT NULL CHECK (kind IN ('conversation', 'answer', 'reply_block')),
  block_id        TEXT,
  reply_block_tag TEXT,
  reply_block_id  TEXT,
  title           TEXT,
  taken_at        INTEGER NOT NULL,
  CHECK ((kind = 'answer') = (block_id IS NOT NULL)),
  CHECK ((kind = 'reply_block') = (reply_block_tag IS NOT NULL)),
  CHECK ((kind = 'reply_block') = (reply_block_id IS NOT NULL)),
  CHECK (kind <> 'reply_block' OR title IS NOT NULL)
) STRICT;
INSERT INTO shares_new SELECT token, user_id, conversation_id, kind, block_id, reply_block_tag, reply_block_id, title, taken_at FROM shares;
DROP TABLE shares;
ALTER TABLE shares_new RENAME TO shares;
CREATE UNIQUE INDEX shares_of_conversation ON shares (user_id, conversation_id) WHERE kind = 'conversation';
CREATE UNIQUE INDEX shares_of_answer ON shares (user_id, conversation_id, block_id) WHERE kind = 'answer';
CREATE UNIQUE INDEX shares_of_reply_block ON shares (user_id, conversation_id, reply_block_tag, reply_block_id)
  WHERE kind = 'reply_block';

CREATE TABLE managed_operations_new (
  device_id    TEXT NOT NULL,
  operation_id TEXT NOT NULL,
  base_version TEXT NOT NULL,
  phase        TEXT NOT NULL
                 CHECK (phase IN ('stopping', 'saving', 'rebuilding', 'booting', 'ready', 'failed')),
  error        TEXT,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (device_id, operation_id)
) STRICT;
INSERT INTO managed_operations_new SELECT device_id, operation_id, base_version, phase, error, updated_at FROM managed_operations;
DROP TABLE managed_operations;
ALTER TABLE managed_operations_new RENAME TO managed_operations;

CREATE TABLE provider_share_keys_new (
  token_hash  TEXT PRIMARY KEY,
  id          TEXT NOT NULL UNIQUE,
  owner_id    TEXT NOT NULL REFERENCES users (id),
  provider_id TEXT NOT NULL,
  created_by  TEXT NOT NULL REFERENCES users (id),
  created_at  INTEGER NOT NULL,
  used_by     TEXT,
  used_at     INTEGER,
  CHECK ((used_by IS NULL) = (used_at IS NULL))
) STRICT;
INSERT INTO provider_share_keys_new
  SELECT k.token_hash, k.id, p.owner_user_id, k.provider_id, k.created_by, k.created_at, k.used_by, k.used_at
    FROM provider_share_keys k JOIN providers p ON p.id = k.provider_id;
DROP TABLE provider_share_keys;
ALTER TABLE provider_share_keys_new RENAME TO provider_share_keys;
CREATE INDEX provider_share_keys_provider ON provider_share_keys (owner_id, provider_id, created_at);
";

/// Writes the core database of `user` in `directory` from the rows of the
/// control database, in place of one a stopped start left.
fn write_core(transaction: &Transaction<'_>, user: &str, directory: &Path) -> rusqlite::Result<()> {
    std::fs::create_dir_all(directory).map_err(corrupt)?;
    let writing = directory.join("core.sqlite.writing");
    remove_file(&writing)?;
    let mut core = Connection::open(&writing)?;
    let version: i32 = demi_core_store::schema::version_of(CORE_SCHEMA);
    let written = core.transaction()?;
    written.execute_batch(CORE_SCHEMA)?;
    for (table, condition) in TABLES {
        copy_rows(transaction, &written, user, table, condition)?;
    }
    // A usage row a media job keeps for its end named its user.
    written.execute_batch("UPDATE media_jobs SET usage = json_remove(usage, '$.user');")?;
    written.pragma_update(None, "user_version", version)?;
    written.commit()?;
    drop(core);
    let path = directory.join("core.sqlite");
    // An earlier run, which a rollback undid, may have left its core's log.
    for sidecar in ["-wal", "-shm"] {
        remove_file(&with_suffix(&path, sidecar))?;
    }
    std::fs::rename(&writing, path).map_err(corrupt)
}

/// Copies `table`'s rows that `condition` picks for `user`, in the columns the
/// core's table has. A core's one-row table numbers its row 1.
fn copy_rows(from: &Connection, to: &Connection, user: &str, table: &str, condition: &str) -> rusqlite::Result<()> {
    let columns: Vec<String> = to
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let held: Vec<String> = from
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let read: Vec<String> = columns
        .iter()
        .map(|column| if held.contains(column) { format!("\"{column}\"") } else { "1".to_owned() })
        .collect();
    let mut rows = from.prepare(&format!("SELECT {} FROM {table} WHERE {condition}", read.join(", ")))?;
    let mut rows = rows.query([user])?;
    let places = vec!["?"; columns.len()].join(", ");
    let names = columns.iter().map(|column| format!("\"{column}\"")).collect::<Vec<_>>().join(", ");
    let mut insert = to.prepare(&format!("INSERT INTO {table} ({names}) VALUES ({places})"))?;
    while let Some(row) = rows.next()? {
        let values = (0..columns.len()).map(|index| row.get::<_, Value>(index)).collect::<rusqlite::Result<Vec<_>>>()?;
        insert.execute(params_from_iter(values))?;
    }
    Ok(())
}

/// Moves the conversation databases of `user`, those of its conversations, of
/// its deletions not finished and of its Forks under way.
fn move_conversations(transaction: &Transaction<'_>, user: &str, data: &Path, directory: &Path) -> rusqlite::Result<()> {
    let to = directory.join("conversations");
    std::fs::create_dir_all(&to).map_err(corrupt)?;
    let ids: Vec<String> = transaction
        .prepare(
            "SELECT id FROM conversations WHERE user_id = ?1
             UNION ALL SELECT id FROM conversation_deletions WHERE user_id = ?1
             UNION ALL SELECT id FROM conversation_fork_operations WHERE user_id = ?1",
        )?
        .query_map([user], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let from = data.join("conversations");
    for id in ids {
        // A conversation's file is named by its id in lower case.
        let file = format!("{}.sqlite", id.to_ascii_lowercase());
        move_database(&from.join(&file), &to.join(&file))?;
    }
    Ok(())
}

/// Moves the database at `from` to `to`, then its log. A database that is
/// not there has moved already or never was, and only a log a stopped run
/// left behind follows it; a log at `to` that an earlier run left, which a
/// rollback undid, goes before the database moves, so it never meets another
/// database.
fn move_database(from: &Path, to: &Path) -> rusqlite::Result<()> {
    if from.exists() {
        for sidecar in ["-wal", "-shm"] {
            remove_file(&with_suffix(to, sidecar))?;
        }
        move_file(from, to)?;
    }
    for sidecar in ["-wal", "-shm"] {
        move_file(&with_suffix(from, sidecar), &with_suffix(to, sidecar))?;
    }
    Ok(())
}

/// `path` with `suffix` after its file name.
fn with_suffix(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// Moves the file at `from` to `to`; one that is not there has moved already
/// or never was.
fn move_file(from: &Path, to: &Path) -> rusqlite::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(corrupt(error)),
    }
}

/// Removes the directory at `path` if it is there and empty.
fn remove_empty_directory(path: &Path) -> rusqlite::Result<()> {
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty) =>
        {
            Ok(())
        }
        Err(error) => Err(corrupt(error)),
    }
}

/// Removes the file at `path`, if there is one.
fn remove_file(path: &Path) -> rusqlite::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(corrupt(error)),
    }
}
