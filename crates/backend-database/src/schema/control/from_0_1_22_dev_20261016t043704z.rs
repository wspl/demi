//! From 0.1.22-dev.20261016T043704Z's control schema, which a server keeps data
//! of, to the next one, in two steps.
//!
//! First, [`CONTROL_FROM_0_1_22_DEV_20261016T043704Z`]:
//!
//! - The usage ledger keeps a row per vendor call
//!   (`crates/backend-database/src/usage.rs`).
//! - A managed device says which managed machine it is.
//! - A Claude Code account may hold no secret.
//!
//! Then where a conversation runs decides its project
//! (`crates/server/src/main.rs`):
//!
//! - A device keeps the home its runner last reported, none until its runner's
//!   next hello.
//! - A Cloud workspace's directory loses a trailing separator, and the user has
//!   one workspace per device and directory, of several the oldest, which the
//!   others' conversations and pending moves move to.
//! - A Cloud workspace at the Cloud's home goes, its conversations running in the
//!   home.
//! - A workspace selection keeps its device and directory.
//! - A Cloud conversation in a directory of its own choosing moves to the Cloud's
//!   home, `/home/demi`, a conversation's directory under
//!   `/home/demi/sessions`, or else the workspace of its directory, made when
//!   there is none.
//! - A permission request of a category and a host at once, which only the
//!   removed `demi conversation move` raised, goes.
//!
//! A paired device's workspaces and conversations wait for its runner to resolve
//! their directories when it first reports its home (`devices.rs`).

use rusqlite::{OptionalExtension, Transaction, params};

use super::super::{Migration, ReadBlob, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_22_dev_20261016t043704z);

fn control_from_0_1_22_dev_20261016t043704z(transaction: &Transaction<'_>, blobs: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(CONTROL_FROM_0_1_22_DEV_20261016T043704Z)?;
    control_by_directory(transaction, blobs)
}

/// The usage ledger's step of [`control_from_0_1_22_dev_20261016t043704z`], to the
/// ledger that keeps a row per vendor call with its agent, turn, purpose,
/// account, ids, outcome, observed quota and times
/// (`crates/backend-database/src/usage.rs`).
///
/// A row written before keeps null for what it lacked; its `created_at`, written
/// as its response ended, is its `ended_at`. A call's counts are null when the
/// vendor reported none.
///
/// In the same schema:
///
/// - A managed device says which managed machine it is, the user's Cloud or the
///   master's Claude Code machine (`crates/backend-cloud/src/lib.rs`). Every
///   managed device of the earlier schema is a Cloud.
/// - A subscription account may hold no secret, as a Claude Code account whose
///   CLI keeps its sign-in does, and says whether the vendor signed it out
///   (`crates/provider-claude-code/src/account.rs`). A Claude Code account keeps
///   the tokens it holds until its first use hands them over.
///
/// No table refers to `provider_credentials`, so it is made again.
const CONTROL_FROM_0_1_22_DEV_20261016T043704Z: &str = "
CREATE TABLE usage_ledger_calls (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title')),
  provider_id           TEXT NOT NULL,
  account_id            TEXT,
  model_id              TEXT NOT NULL,
  request_id            TEXT,
  vendor_id             TEXT,
  input_tokens          INTEGER,
  output_tokens         INTEGER,
  cache_read_tokens     INTEGER,
  cache_write_tokens    INTEGER,
  cache_write_1h_tokens INTEGER,
  outcome               TEXT,
  quota                 TEXT,
  started_at            INTEGER,
  ended_at              INTEGER NOT NULL
) STRICT;
INSERT INTO usage_ledger_calls (id, user_id, conversation_id, provider_id, model_id,
    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, ended_at)
  SELECT id, user_id, conversation_id, provider_id, model_id,
    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at
  FROM usage_ledger ORDER BY rowid;
DROP TABLE usage_ledger;
ALTER TABLE usage_ledger_calls RENAME TO usage_ledger;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);
ALTER TABLE devices ADD COLUMN machine TEXT CHECK (machine IN ('cloud', 'claude_code'));
UPDATE devices SET machine = 'cloud' WHERE kind = 'managed';
DROP INDEX devices_one_managed;
CREATE UNIQUE INDEX devices_one_machine ON devices (user_id, machine) WHERE kind = 'managed';
CREATE TABLE provider_credentials_new (
  provider_id  TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  id           TEXT NOT NULL,
  identity_key TEXT,
  label        TEXT NOT NULL,
  detail       TEXT,
  source       TEXT NOT NULL,
  secret       BLOB,
  version      INTEGER NOT NULL CHECK (version >= 1),
  quota        TEXT,
  updated_at   INTEGER NOT NULL,
  position     INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0),
  signed_out   INTEGER NOT NULL DEFAULT 0 CHECK (signed_out IN (0, 1)),
  PRIMARY KEY (provider_id, id)
) STRICT;
INSERT INTO provider_credentials_new
  (provider_id, id, identity_key, label, detail, source, secret, version, quota, updated_at, position)
  SELECT provider_id, id, identity_key, label, detail, source, secret, version, quota, updated_at, position
  FROM provider_credentials;
DROP TABLE provider_credentials;
ALTER TABLE provider_credentials_new RENAME TO provider_credentials;
CREATE UNIQUE INDEX provider_credentials_identity ON provider_credentials (provider_id, identity_key)
  WHERE identity_key IS NOT NULL;
";

/// The projects-by-directory step of
/// [`control_from_0_1_22_dev_20261016t043704z`].
fn control_by_directory(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
ALTER TABLE devices ADD COLUMN home TEXT;
-- A Cloud's directory as its Host names it, without a trailing separator; a
-- paired device's waits for its runner (devices.rs, settle_first_home).
UPDATE workspaces SET path = CASE WHEN rtrim(path, '/') = '' THEN '/' ELSE rtrim(path, '/') END
  WHERE device_id IN (SELECT id FROM devices WHERE kind = 'managed');
-- Workspaces of one directory become the oldest of them.
CREATE TEMP TABLE kept_workspaces AS
  SELECT w.id AS id, (SELECT k.id FROM workspaces k
      WHERE k.user_id = w.user_id AND k.device_id = w.device_id AND k.path = w.path
      ORDER BY k.created_at, k.id LIMIT 1) AS kept
  FROM workspaces w;
UPDATE conversations SET target_workspace_id = (SELECT kept FROM kept_workspaces WHERE id = target_workspace_id)
  WHERE target_workspace_id IS NOT NULL;
UPDATE conversations SET pending_workspace_id = (SELECT kept FROM kept_workspaces WHERE id = pending_workspace_id)
  WHERE pending_workspace_id IS NOT NULL;
DELETE FROM workspaces WHERE id IN (SELECT id FROM kept_workspaces WHERE kept <> id);
DROP TABLE kept_workspaces;
CREATE UNIQUE INDEX workspaces_directory ON workspaces (user_id, device_id, path);
",
    )?;
    // The table's CHECK changes.
    //
    // Making the table again would drop the table every conversation's rows name,
    // which cascades while foreign keys are on, as they are in a migration's
    // transaction. Its columns stay as they are, so its text changes in place, the
    // procedure SQLite gives for a change of constraints alone
    // (lang_altertable.html § 7), to 0.1.44's text, whose columns it has and
    // which the later steps start from.
    //
    // The rows the new CHECK refuses are rewritten next, and the integrity check,
    // which tests CHECK constraints, proves none is left.
    let shipped = include_str!("from_0_1_44.sql");
    let start = shipped
        .find("CREATE TABLE conversations (")
        .expect("the control schema makes the conversations table");
    let end = start + shipped[start..].find(") STRICT;").expect("the table ends") + ") STRICT".len();
    let version: i64 = transaction.pragma_query_value(None, "schema_version", |row| row.get(0))?;
    transaction.pragma_update(None, "writable_schema", true)?;
    transaction.execute(
        "UPDATE sqlite_schema SET sql = ?1 WHERE type = 'table' AND name = 'conversations'",
        [&shipped[start..end]],
    )?;
    transaction.pragma_update(None, "schema_version", version + 1)?;
    transaction.pragma_update(None, "writable_schema", false)?;
    transaction.execute_batch(
        "
UPDATE conversations SET
    target_device_id = (SELECT device_id FROM workspaces WHERE id = target_workspace_id),
    target_path = (SELECT path FROM workspaces WHERE id = target_workspace_id)
  WHERE target_kind = 'workspace';
-- A Cloud workspace at the Cloud's home goes: its conversations run in the
-- home, outside projects, and a move into it waits as a move to the home.
CREATE TEMP TABLE cloud_homes AS
  SELECT w.id AS id, w.device_id AS device_id FROM workspaces w JOIN devices d ON d.id = w.device_id
  WHERE d.kind = 'managed' AND w.path = '/home/demi';
UPDATE conversations SET target_kind = 'home', target_path = NULL, target_workspace_id = NULL
  WHERE target_workspace_id IN (SELECT id FROM cloud_homes);
UPDATE conversations SET pending_kind = 'cloud', pending_path = '/home/demi', pending_workspace_id = NULL
  WHERE pending_workspace_id IN (SELECT id FROM cloud_homes);
DELETE FROM workspaces WHERE id IN (SELECT id FROM cloud_homes);
DROP TABLE cloud_homes;
-- A permission request asks for one category or one host now. Only
-- `demi conversation move`, which this release removes, asked for a
-- category and a host at once; such a request goes, as the command did.
DELETE FROM permission_requests WHERE kind = 'permission'
  AND json_array_length(categories) + (host_device_id IS NOT NULL) + (pairing_code IS NOT NULL) <> 1;
",
    )?;
    let moving = transaction
        .prepare("SELECT id, user_id, target_path FROM conversations WHERE target_kind = 'cloud' AND target_path IS NOT NULL")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let now = jiff::Timestamp::now().as_millisecond();
    for (id, owner, path) in moving {
        let cloud = cloud_device_of_0_1_22(transaction, &owner, now)?;
        select_cloud_directory_of_0_1_22(transaction, &id, &owner, &cloud, &path, now)?;
    }
    let problems: Vec<String> = transaction
        .prepare("PRAGMA integrity_check")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if problems != ["ok"] {
        return Err(corrupt_text(&format!("the conversations' targets fail their check: {problems:?}")));
    }
    Ok(())
}

/// The selection of the Cloud directory `path` for `owner`'s conversation
/// `id`, as the rule of 0.1.22 made it and its columns kept it, the Cloud's
/// home being `/home/demi` until its runner reported one: the conversation's
/// directory of its own or another session's, the home, or else the user's
/// workspace of the directory, made when there was none.
fn select_cloud_directory_of_0_1_22(
    transaction: &Transaction<'_>,
    id: &str,
    owner: &str,
    cloud: &str,
    path: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let home = "/home/demi";
    let path = match path.trim_end_matches('/') {
        "" => "/",
        trimmed => trimmed,
    };
    let sessions = format!("{home}/sessions");
    if path.rsplit_once('/').map(|(parent, _)| parent) == Some(sessions.as_str()) {
        let own = format!("{sessions}/{id}");
        transaction.execute(
            "UPDATE conversations SET target_kind = 'cloud', target_device_id = NULL, target_path = ?2,
               target_workspace_id = NULL WHERE id = ?1",
            params![id, (path != own).then_some(path)],
        )?;
        return Ok(());
    }
    if path == home {
        transaction.execute(
            "UPDATE conversations SET target_kind = 'home', target_device_id = ?2, target_path = NULL,
               target_workspace_id = NULL WHERE id = ?1",
            params![id, cloud],
        )?;
        return Ok(());
    }
    let existing: Option<String> = transaction
        .query_row(
            "SELECT id FROM workspaces WHERE user_id = ?1 AND device_id = ?2 AND path = ?3",
            params![owner, cloud, path],
            |row| row.get(0),
        )
        .optional()?;
    let workspace = match existing {
        Some(workspace) => workspace,
        None => {
            let workspace = uuid::Uuid::new_v4().to_string();
            let name = path.rsplit('/').find(|part| !part.is_empty()).unwrap_or(path);
            transaction.execute(
                "INSERT INTO workspaces (id, user_id, device_id, path, name, sort_order, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5,
                   (SELECT COALESCE(MIN(sort_order), 1) - 1 FROM workspaces WHERE user_id = ?2), ?6)",
                params![workspace, owner, cloud, path, name, now],
            )?;
            workspace
        }
    };
    transaction.execute(
        "UPDATE conversations SET target_kind = 'workspace', target_device_id = ?2, target_path = ?3,
           target_workspace_id = ?4 WHERE id = ?1",
        params![id, cloud, path, workspace],
    )?;
    Ok(())
}

/// The ID of `owner`'s Cloud device, made now when it was never made, as
/// the devices table of this step kept it.
fn cloud_device_of_0_1_22(transaction: &Transaction<'_>, owner: &str, now: i64) -> rusqlite::Result<String> {
    transaction.execute(
        "INSERT INTO devices (id, user_id, kind, name, platform, token_hash, claimed_at, last_seen_at, machine)
         VALUES (?1, ?2, 'managed', 'Cloud', 'linux', NULL, ?3, NULL, 'cloud')
         ON CONFLICT DO NOTHING",
        params![uuid::Uuid::new_v4().to_string(), owner, now],
    )?;
    transaction.query_row(
        "SELECT id FROM devices WHERE kind = 'managed' AND user_id = ?1 AND machine = 'cloud'",
        [owner],
        |row| row.get(0),
    )
}
