//! From 0.1.5614's control schema to 0.1.5658's: the server settings'
//! part ([`CONTROL_SERVER_SETTINGS_FROM_0_1_5614`]), then a project's tables,
//! columns, indexes and values say project where they said workspace, and an
//! account holds workspaces.
//!
//! - The tables and columns are renamed, which keeps their rows, and their
//!   indexes made again under the new names.
//! - Each user gets one workspace, with the name an account starts with, and
//!   every conversation and project of theirs is in it.
//! - A repository's worktrees are kept by the device and directory of their
//!   project, since one directory may now be a project in several
//!   workspaces.
//! - A table's text then becomes 0.1.5658's, comments included: a
//!   change of the text alone, which the new CHECK of `target_kind`, the new
//!   columns' constraints and the conversations' key of their project need,
//!   as SQLite's procedure for a change of constraints gives it
//!   (lang_altertable.html § 7). The values the new CHECK, the stored
//!   switches and the Forks' targets name are rewritten next. The integrity
//!   check, which tests CHECK and NOT NULL constraints, and the foreign key
//!   check prove none is left.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_5614);

/// The schema this step leads to, whose tables it takes.
const NEXT: &str = include_str!("from_0_1_5658.sql");

fn control_from_0_1_5614(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(CONTROL_SERVER_SETTINGS_FROM_0_1_5614)?;
    transaction.execute_batch(
        "
ALTER TABLE workspaces RENAME TO projects;
ALTER TABLE saved_messages RENAME COLUMN workspace_id TO project_id;
ALTER TABLE conversations RENAME COLUMN target_workspace_id TO target_project_id;
DROP INDEX workspaces_order;
DROP INDEX workspaces_directory;
DROP INDEX conversations_workspace;
DROP INDEX saved_messages_of_workspace;
",
    )?;
    for table in ["workspaces", "directory_worktrees"] {
        transaction.execute_batch(&format!("{};", table_in(NEXT, table)))?;
    }
    transaction.execute_batch(
        "
CREATE INDEX workspaces_order ON workspaces (user_id, sort_order);
CREATE INDEX directory_worktrees_path ON directory_worktrees (device_id, worktree_path);
INSERT INTO workspaces (id, user_id, name, icon, color, sort_order, created_at)
  SELECT lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4'
           || substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', 1 + abs(random()) % 4, 1)
           || substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))),
         id, NULL, 'user', 'blue', 0, CAST(unixepoch('subsec') * 1000 AS INTEGER)
  FROM users;
ALTER TABLE projects ADD COLUMN workspace_id TEXT;
UPDATE projects SET workspace_id = (SELECT id FROM workspaces WHERE workspaces.user_id = projects.user_id);
ALTER TABLE conversations ADD COLUMN workspace_id TEXT;
UPDATE conversations SET workspace_id = (SELECT id FROM workspaces WHERE workspaces.user_id = conversations.user_id);
-- A release before projects were kept when deleted removed a project and
-- left its archived conversations naming it. Such a conversation names the
-- project of its directory in its workspace where there is one, or else that
-- project again, deleted, which its restore brings back; without its device
-- it runs in its directory on the device, as one whose device went does.
UPDATE conversations SET target_project_id = (
    SELECT p.id FROM projects p
    WHERE p.workspace_id = conversations.workspace_id AND p.device_id = conversations.target_device_id
      AND p.path = conversations.target_path)
  WHERE target_project_id IS NOT NULL AND target_project_id NOT IN (SELECT id FROM projects)
    AND EXISTS (SELECT 1 FROM projects p
      WHERE p.workspace_id = conversations.workspace_id AND p.device_id = conversations.target_device_id
        AND p.path = conversations.target_path);
INSERT INTO projects (id, user_id, workspace_id, device_id, path, sort_order, created_at, deleted_at)
  SELECT target_project_id, user_id, workspace_id, target_device_id, target_path, 0,
         min(created_at), CAST(unixepoch('subsec') * 1000 AS INTEGER)
  FROM conversations
  WHERE target_project_id IS NOT NULL AND target_project_id NOT IN (SELECT id FROM projects)
    AND target_device_id IN (SELECT id FROM devices)
  GROUP BY target_project_id;
UPDATE conversations SET target_kind = 'device', target_project_id = NULL,
    target_worktree_name = NULL, target_worktree_path = NULL
  WHERE target_project_id IS NOT NULL AND target_project_id NOT IN (SELECT id FROM projects);
INSERT INTO directory_worktrees (device_id, path, name, worktree_path)
  SELECT p.device_id, p.path, w.name, w.path FROM workspace_worktrees w JOIN projects p ON p.id = w.workspace_id
  WHERE true ON CONFLICT DO NOTHING;
DROP TABLE workspace_worktrees;
CREATE INDEX projects_order ON projects (user_id, sort_order);
CREATE UNIQUE INDEX projects_directory ON projects (workspace_id, device_id, path);
CREATE UNIQUE INDEX projects_of_workspace ON projects (id, workspace_id);
CREATE INDEX conversations_project ON conversations (target_project_id);
CREATE INDEX conversations_of_workspace ON conversations (workspace_id);
CREATE INDEX saved_messages_of_project ON saved_messages (project_id, saved_at);
",
    )?;
    let version: i64 = transaction.pragma_query_value(None, "schema_version", |row| row.get(0))?;
    transaction.pragma_update(None, "writable_schema", true)?;
    for table in ["projects", "conversations", "saved_messages"] {
        transaction.execute(
            "UPDATE sqlite_schema SET sql = ?1 WHERE type = 'table' AND name = ?2",
            [table_in(NEXT, table), table],
        )?;
    }
    transaction.pragma_update(None, "schema_version", version + 1)?;
    transaction.pragma_update(None, "writable_schema", false)?;
    transaction.execute_batch(
        "
UPDATE conversations SET target_kind = 'project' WHERE target_kind = 'workspace';
UPDATE conversations SET last_switch = json_set(json_remove(last_switch, '$.from.workspaceId'),
    '$.from.kind', 'project', '$.from.projectId', json_extract(last_switch, '$.from.workspaceId'))
  WHERE json_extract(last_switch, '$.from.kind') = 'workspace';
UPDATE conversations SET last_switch = json_set(json_remove(last_switch, '$.to.workspaceId'),
    '$.to.kind', 'project', '$.to.projectId', json_extract(last_switch, '$.to.workspaceId'))
  WHERE json_extract(last_switch, '$.to.kind') = 'workspace';
UPDATE conversation_fork_operations SET metadata = json_set(json_remove(metadata, '$.target.workspaceId'),
    '$.target.kind', 'project', '$.target.projectId', json_extract(metadata, '$.target.workspaceId'))
  WHERE json_extract(metadata, '$.target.kind') = 'workspace';
",
    )?;
    let orphans: i64 = transaction.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| row.get(0))?;
    if orphans != 0 {
        return Err(corrupt_text(&format!("{orphans} rows name a workspace or project they are not in")));
    }
    let problems: Vec<String> = transaction
        .prepare("PRAGMA integrity_check")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if problems != ["ok"] {
        return Err(corrupt_text(&format!("the projects and workspaces fail their check: {problems:?}")));
    }
    Ok(())
}

/// The server settings' part of [`control_from_0_1_5614`]: they keep the
/// instance mode, which a server that never chose one has as isolated, and an
/// upgrade channel no one chose is null in a row that holds a mode.
///
/// The table is rebuilt, since SQLite cannot make a column nullable in place.
const CONTROL_SERVER_SETTINGS_FROM_0_1_5614: &str = "
CREATE TABLE server_settings_next (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  upgrade_channel TEXT CHECK (upgrade_channel IN ('beta', 'canary')),
  instance_mode   TEXT NOT NULL DEFAULT 'isolated' CHECK (instance_mode IN ('shared', 'isolated'))
) STRICT;
INSERT INTO server_settings_next (id, upgrade_channel)
  SELECT id, upgrade_channel FROM server_settings;
DROP TABLE server_settings;
ALTER TABLE server_settings_next RENAME TO server_settings;
";

/// The text SQLite keeps of `table` in the schema `sql`: from its `CREATE
/// TABLE` to the end of its `) STRICT`.
fn table_in<'a>(sql: &'a str, table: &str) -> &'a str {
    let start = sql
        .find(&format!("CREATE TABLE {table} ("))
        .expect("the schema makes the table");
    let end = start + sql[start..].find(") STRICT;").expect("the table ends") + ") STRICT".len();
    &sql[start..end]
}
