//! From 0.1.22's control schema to 0.1.25's.
//!
//! - A usage ledger row may be an image's, which SQLite's checks let in only on a
//!   table made again, its rows as they were.
//! - Each browser's push subscription is kept.
//! - A running job may wait to be sent to its device
//!   (`sessions-and-targets.md` § Host operations), which every one recorded
//!   before was.
//! - The users' automations and their runs, none before.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_22);

const CONTROL_FROM_0_1_22: &str = "
ALTER TABLE running_jobs ADD COLUMN sent INTEGER NOT NULL DEFAULT 1 CHECK (sent IN (0, 1));
CREATE TABLE usage_ledger_images (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  -- Null for a request outside a session, such as a title.
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title', 'image')),
  provider_id           TEXT NOT NULL,
  -- Null for an API key.
  account_id            TEXT,
  model_id              TEXT NOT NULL,
  request_id            TEXT,
  vendor_id             TEXT,
  input_tokens          INTEGER,
  output_tokens         INTEGER,
  cache_read_tokens     INTEGER,
  cache_write_tokens    INTEGER,
  cache_write_1h_tokens INTEGER,
  -- 'answered', 'cancelled' or the failure code of a refused call.
  outcome               TEXT,
  quota                 TEXT,
  started_at            INTEGER,
  ended_at              INTEGER NOT NULL
) STRICT;
INSERT INTO usage_ledger_images SELECT * FROM usage_ledger ORDER BY rowid;
DROP TABLE usage_ledger;
ALTER TABLE usage_ledger_images RENAME TO usage_ledger;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);
-- A browser's push subscription of a user (turns.md § Notifications), one
-- per session, which it serves and which takes it along when it ends: its
-- push service's endpoint, which names the browser's subscription, its two
-- keys and the events it chose.
CREATE TABLE push_subscriptions (
  session_id       TEXT PRIMARY KEY REFERENCES web_sessions (id) ON DELETE CASCADE,
  endpoint         TEXT NOT NULL UNIQUE,
  user_id          TEXT NOT NULL REFERENCES users (id),
  p256dh           TEXT NOT NULL,
  auth             TEXT NOT NULL,
  turn_finishes    INTEGER NOT NULL CHECK (turn_finishes IN (0, 1)),
  turn_fails       INTEGER NOT NULL CHECK (turn_fails IN (0, 1)),
  needs_permission INTEGER NOT NULL CHECK (needs_permission IN (0, 1)),
  agent_notifies   INTEGER NOT NULL CHECK (agent_notifies IN (0, 1)),
  created_at       INTEGER NOT NULL
) STRICT;
CREATE INDEX push_subscriptions_of_user ON push_subscriptions (user_id);
-- Each user's automations (`automations.md`): the trigger as JSON,
-- automation trigger format 1, the `Trigger` of the web contract; the
-- target as JSON, automation target format 1, with the existing
-- conversation it wakes again in `conversation_id`, null for a new one each
-- run; the model settings as JSON, null for none chosen; why it paused
-- itself, null while it is on or the user paused it; when its trigger was
-- last set or it was last turned on, before which no time of its schedule
-- fires; a URL trigger's secret and the Stripe signing secret the user
-- pasted, each sealed with the automation as the additional data; and a
-- change watch's last output, its standard output's last 16 KB, the
-- SHA-256 of the whole of it, which a check compares, and its exit status,
-- with when its command last ran, all null before its first.
CREATE TABLE automations (
  id               TEXT PRIMARY KEY,
  user_id          TEXT NOT NULL REFERENCES users (id),
  name             TEXT NOT NULL,
  trigger          TEXT NOT NULL,
  time_zone        TEXT NOT NULL,
  max_runs_per_day INTEGER CHECK (max_runs_per_day > 0),
  target           TEXT NOT NULL,
  conversation_id  TEXT,
  prompt           TEXT NOT NULL,
  model            TEXT,
  paused           INTEGER NOT NULL CHECK (paused IN (0, 1)),
  paused_because   TEXT CHECK (paused_because IN ('conversation_archived', 'conversation_deleted')),
  schedule_since   INTEGER NOT NULL,
  secret           BLOB,
  stripe_secret    BLOB,
  watch_stdout     BLOB,
  watch_digest     BLOB,
  watch_exit       INTEGER,
  watch_checked_at INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
) STRICT;
CREATE INDEX automations_by_user ON automations (user_id, created_at);
CREATE INDEX automations_by_conversation ON automations (conversation_id) WHERE conversation_id IS NOT NULL;
-- Each automation's runs (`automations.md` § Runs): what fired it, when,
-- its state with a note as JSON, automation run note format 1, the `RunNote`
-- of the web contract, the start of its agent's last answer, the
-- conversation it ran in with the turn its message started, whether that
-- new conversation is still filed under the automation, out of the
-- sidebar, whether it notified and its conversation was not opened since,
-- and whether its agent notified at all, which makes it end Notified. A
-- run's conversation outlives it.
CREATE TABLE automation_runs (
  id              TEXT PRIMARY KEY,
  automation_id   TEXT NOT NULL REFERENCES automations (id) ON DELETE CASCADE,
  cause           TEXT NOT NULL CHECK (cause IN ('time', 'change', 'url', 'now')),
  at              INTEGER NOT NULL,
  state           TEXT NOT NULL CHECK (state IN ('running', 'done', 'notified', 'skipped', 'failed')),
  note            TEXT,
  line            TEXT,
  conversation_id TEXT,
  turn_id         TEXT,
  filed           INTEGER NOT NULL CHECK (filed IN (0, 1)),
  unread          INTEGER NOT NULL CHECK (unread IN (0, 1)),
  notified        INTEGER NOT NULL CHECK (notified IN (0, 1))
) STRICT;
CREATE INDEX automation_runs_by_automation ON automation_runs (automation_id, at);
CREATE INDEX automation_runs_by_conversation ON automation_runs (conversation_id) WHERE conversation_id IS NOT NULL;
-- A URL's calls (`automations.md` § Triggers), each kept as it is taken,
-- with its delivery ID: pending until a run takes it, so a restart loses
-- none; taken; or held, for one a paused automation or its daily limit
-- answered and one a pause or an edit dropped. A pending call keeps its
-- method, its headers as JSON and its body; the others keep only what
-- dedupes a repeated delivery ID, for an hour.
CREATE TABLE automation_calls (
  id            INTEGER PRIMARY KEY,
  automation_id TEXT NOT NULL REFERENCES automations (id) ON DELETE CASCADE,
  at            INTEGER NOT NULL,
  delivery_id   TEXT,
  state         TEXT NOT NULL CHECK (state IN ('pending', 'taken', 'held')),
  method        TEXT,
  headers       TEXT,
  body          BLOB,
  CHECK ((state = 'pending') = (method IS NOT NULL AND headers IS NOT NULL AND body IS NOT NULL))
) STRICT;
CREATE INDEX automation_calls_by_state ON automation_calls (automation_id, state, id);
CREATE INDEX automation_calls_by_delivery ON automation_calls (automation_id, delivery_id)
  WHERE delivery_id IS NOT NULL;
";
