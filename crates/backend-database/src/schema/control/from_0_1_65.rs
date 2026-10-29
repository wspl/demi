//! From 0.1.65's control schema, which 0.1.64 shipped too, to 0.1.5409's.
//!
//! - An automation that wakes an existing conversation becomes that
//!   conversation's schedule, with its runs and its pending URL calls, its URL
//!   secret sealed again as a schedule's. Its trigger names its time `timing`,
//!   not `schedule`.
//! - An automation that starts a new conversation each run is deleted with its
//!   runs and calls; the conversations its runs started stay and join the
//!   sidebar, since only an automation's run filed a conversation out of it.
//!   One whose conversation is gone, or that paused for its deletion, goes the
//!   same way.
//! - The Stripe signing secret a user pasted goes, and so do the runs that
//!   skipped for a conversation gone or a model not chosen, which no note
//!   says now.
//! - An archived conversation's schedules do not wake it, read from its
//!   archived state, so no schedule is paused for it: one an archiving paused
//!   is on while its conversation is still archived, counting from the
//!   migration, as a schedule turned on does. One whose conversation was
//!   restored since stays paused, as its user left it: a restore turned
//!   nothing on.
//! - A schedule names its conversation as the conversation's record spells
//!   it, and only a conversation of the automation's user.
//! - The permission to manage automations goes, with its grants and every
//!   request for it still stored, since an agent manages its conversation's
//!   schedules without asking.
//! - The permission to organize conversations goes, with its grants and every
//!   request for it still stored: one that waits for an answer, and one whose
//!   answer a restart kept from its agent, which would reach the agent with
//!   the category's id for its words. An agent renames its conversation
//!   without asking.
//! - The saved shortcuts are the Mac's set, and the PC's starts at its
//!   defaults ([`CONTROL_SHORTCUTS_FROM_0_1_65`]).
//! - A conversation in a linked worktree of its project keeps the worktree
//!   with its selection, and each project the worktrees its Host reports
//!   ([`CONTROL_WORKTREES_FROM_0_1_65`]).
//!
//! The automations are read one row at a time: the table is small, but a
//! migration collects no table into memory.

use rusqlite::{OptionalExtension, Transaction};

use crate::providers::ResealSealed;
use super::super::{Migration, corrupt, corrupt_text};
use super::shared::{named_timing};

pub(super) const MIGRATION: Migration = Migration::Reseal(control_from_0_1_65);

fn control_from_0_1_65(transaction: &Transaction<'_>, seals: &dyn ResealSealed) -> rusqlite::Result<()> {
    transaction.execute_batch(CONTROL_SHORTCUTS_FROM_0_1_65)?;
    transaction.execute_batch(CONTROL_WORKTREES_FROM_0_1_65)?;
    transaction.execute_batch(CONTROL_SCHEDULES_FROM_0_1_65)?;
    {
        let mut read = transaction.prepare(
            "SELECT id, user_id, name, trigger, time_zone, max_runs_per_day, target, prompt, paused, paused_because,
                    schedule_since, secret, watch_stdout, watch_digest, watch_exit, watch_checked_at,
                    created_at, updated_at
             FROM automations ORDER BY created_at, id",
        )?;
        let mut live = transaction.prepare("SELECT id, archived FROM conversations WHERE id = ?1 AND user_id = ?2")?;
        let mut insert = transaction.prepare(
            "INSERT INTO schedules (id, conversation_id, name, trigger, time_zone, max_runs_per_day, if_busy,
               prompt, paused, since, secret, watch_stdout, watch_digest, watch_exit,
               watch_checked_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )?;
        let mut rows = read.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get("id")?;
            let target: serde_json::Value = serde_json::from_str(&row.get::<_, String>("target")?).map_err(corrupt)?;
            let paused_because: Option<String> = row.get("paused_because")?;
            let (Some("existing"), Some(named)) = (
                target.get("kind").and_then(serde_json::Value::as_str),
                target.get("conversationId").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            if paused_because.as_deref() == Some("conversation_deleted") {
                continue;
            }
            let user: String = row.get("user_id")?;
            let Some((conversation, archived)) = live
                .query_row(rusqlite::params![named, user], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)))
                .optional()?
            else {
                continue;
            };
            let archive_paused = archived && paused_because.as_deref() == Some("conversation_archived");
            let if_busy = target.get("ifBusy").and_then(serde_json::Value::as_str).unwrap_or("queue");
            let trigger = schedule_trigger_3(&row.get::<_, String>("trigger")?)?;
            let secret = row
                .get::<_, Option<Vec<u8>>>("secret")?
                .map(|sealed| {
                    seals
                        .schedule_secret(&id, &sealed)
                        .map_err(|reason| corrupt_text(&format!("automation {id}: {reason}")))
                })
                .transpose()?;
            insert.execute(rusqlite::params![
                id,
                conversation,
                row.get::<_, String>("name")?,
                trigger.to_string(),
                row.get::<_, String>("time_zone")?,
                row.get::<_, Option<i64>>("max_runs_per_day")?,
                if_busy,
                row.get::<_, String>("prompt")?,
                row.get::<_, bool>("paused")? && !archive_paused,
                if archive_paused { jiff::Timestamp::now().as_millisecond() } else { row.get::<_, i64>("schedule_since")? },
                secret,
                row.get::<_, Option<Vec<u8>>>("watch_stdout")?,
                row.get::<_, Option<Vec<u8>>>("watch_digest")?,
                row.get::<_, Option<i64>>("watch_exit")?,
                row.get::<_, Option<i64>>("watch_checked_at")?,
                row.get::<_, i64>("created_at")?,
                row.get::<_, i64>("updated_at")?,
            ])?;
        }
    }
    transaction.execute_batch(
        "INSERT INTO schedule_runs (id, schedule_id, cause, at, state, note, line, turn_id, notified, task_id)
           SELECT id, automation_id, cause, at, state, note, line, turn_id, notified, task_id
           FROM automation_runs
           WHERE automation_id IN (SELECT id FROM schedules)
             AND (note IS NULL OR json_extract(note, '$.kind') NOT IN ('target_gone', 'no_model'));
         INSERT INTO schedule_calls (id, schedule_id, at, delivery_id, state, method, headers, body)
           SELECT id, automation_id, at, delivery_id, state, method, headers, body
           FROM automation_calls
           WHERE automation_id IN (SELECT id FROM schedules);
         DROP TABLE automation_calls;
         DROP TABLE automation_runs;
         DROP TABLE automations;
         DELETE FROM permission_grants WHERE category IN ('conversation.organize', 'automation.manage');
         DELETE FROM permission_requests
           WHERE kind = 'permission'
             AND EXISTS (SELECT 1 FROM json_each(categories) WHERE value IN ('conversation.organize', 'automation.manage'));",
    )
}

/// The tables of the schedules, as the next schema makes them.
const CONTROL_SCHEDULES_FROM_0_1_65: &str = "
CREATE TABLE schedules (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  name             TEXT NOT NULL,
  trigger          TEXT NOT NULL,
  time_zone        TEXT NOT NULL,
  max_runs_per_day INTEGER CHECK (max_runs_per_day > 0),
  if_busy          TEXT NOT NULL CHECK (if_busy IN ('queue', 'steer')),
  prompt           TEXT NOT NULL,
  paused           INTEGER NOT NULL CHECK (paused IN (0, 1)),
  since            INTEGER NOT NULL,
  secret           BLOB,
  watch_stdout     BLOB,
  watch_digest     BLOB,
  watch_exit       INTEGER,
  watch_checked_at INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
) STRICT;
CREATE INDEX schedules_by_conversation ON schedules (conversation_id, created_at);
CREATE TABLE schedule_runs (
  id          TEXT PRIMARY KEY,
  schedule_id TEXT NOT NULL REFERENCES schedules (id) ON DELETE CASCADE,
  cause       TEXT NOT NULL CHECK (cause IN ('time', 'change', 'url', 'now')),
  at          INTEGER NOT NULL,
  state       TEXT NOT NULL CHECK (state IN ('running', 'done', 'notified', 'skipped', 'failed')),
  note        TEXT,
  line        TEXT,
  turn_id     TEXT,
  notified    INTEGER NOT NULL CHECK (notified IN (0, 1)),
  task_id     TEXT
) STRICT;
CREATE INDEX schedule_runs_by_schedule ON schedule_runs (schedule_id, at);
CREATE TABLE schedule_calls (
  id          INTEGER PRIMARY KEY,
  schedule_id TEXT NOT NULL REFERENCES schedules (id) ON DELETE CASCADE,
  at          INTEGER NOT NULL,
  delivery_id TEXT,
  state       TEXT NOT NULL CHECK (state IN ('pending', 'taken', 'held')),
  method      TEXT,
  headers     TEXT,
  body        BLOB,
  CHECK ((state = 'pending') = (method IS NOT NULL AND headers IS NOT NULL AND body IS NOT NULL))
) STRICT;
CREATE INDEX schedule_calls_by_state ON schedule_calls (schedule_id, state, id);
CREATE INDEX schedule_calls_by_delivery ON schedule_calls (schedule_id, delivery_id)
  WHERE delivery_id IS NOT NULL;
";

/// The shortcuts' part of [`control_from_0_1_65`]: the saved shortcuts are
/// the Mac's set, since all of them are written in its notation; the PC's set
/// starts empty, at its defaults.
const CONTROL_SHORTCUTS_FROM_0_1_65: &str = "
UPDATE user_preferences
   SET preferences = json_set(preferences, '$.shortcuts', json_object('mac', json_extract(preferences, '$.shortcuts'), 'pc', json_object()))
 WHERE json_type(preferences, '$.shortcuts') = 'object';
";

/// The worktrees' part of [`control_from_0_1_65`]: none is known before the
/// Host's first report after the upgrade, and no workflow choice was a
/// worktree's file.
const CONTROL_WORKTREES_FROM_0_1_65: &str = "
ALTER TABLE conversations ADD COLUMN target_worktree_name TEXT;
ALTER TABLE conversations ADD COLUMN target_worktree_path TEXT
  CHECK ((target_worktree_name IS NULL) = (target_worktree_path IS NULL)
    AND (target_worktree_path IS NULL OR target_kind = 'workspace'));
ALTER TABLE conversations ADD COLUMN workflow_worktree INTEGER NOT NULL DEFAULT 0
  CHECK (workflow_worktree IN (0, 1) AND (workflow_worktree = 0 OR workflow_kind = 'workflow'));
CREATE TABLE workspace_worktrees (
  workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
  name         TEXT NOT NULL,
  path         TEXT NOT NULL,
  PRIMARY KEY (workspace_id, name)
) STRICT;
CREATE INDEX workspace_worktrees_path ON workspace_worktrees (path);
";

/// A trigger of format 2 as format 3 holds it: a time trigger names its time
/// `timing`, the name it had as `schedule` while a trigger was not a schedule's.
/// A trigger already in format 3 is read as it is.
fn schedule_trigger_3(text: &str) -> rusqlite::Result<serde_json::Value> {
    let mut value: serde_json::Value = serde_json::from_str(text).map_err(corrupt)?;
    named_timing(&mut value);
    Ok(value)
}
