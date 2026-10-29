//! From 0.1.5672's control schema to the current one: a push
//! subscription's `quiet_while_elsewhere` has no default. The step that
//! added the column gave it one, which only a database migrated through
//! that step kept, so the table is made anew without it.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5672);

const CONTROL_FROM_0_1_5672: &str = "
CREATE TABLE push_subscriptions_next (
  session_id       TEXT PRIMARY KEY REFERENCES web_sessions (id) ON DELETE CASCADE,
  endpoint         TEXT NOT NULL UNIQUE,
  user_id          TEXT NOT NULL REFERENCES users (id),
  p256dh           TEXT NOT NULL,
  auth             TEXT NOT NULL,
  turn_finishes    INTEGER NOT NULL CHECK (turn_finishes IN (0, 1)),
  turn_fails       INTEGER NOT NULL CHECK (turn_fails IN (0, 1)),
  needs_permission INTEGER NOT NULL CHECK (needs_permission IN (0, 1)),
  agent_notifies   INTEGER NOT NULL CHECK (agent_notifies IN (0, 1)),
  created_at       INTEGER NOT NULL,
  quiet_while_elsewhere INTEGER NOT NULL CHECK (quiet_while_elsewhere IN (0, 1))
) STRICT;
INSERT INTO push_subscriptions_next
  SELECT session_id, endpoint, user_id, p256dh, auth, turn_finishes, turn_fails, needs_permission,
    agent_notifies, created_at, quiet_while_elsewhere
  FROM push_subscriptions;
DROP TABLE push_subscriptions;
ALTER TABLE push_subscriptions_next RENAME TO push_subscriptions;
CREATE INDEX push_subscriptions_of_user ON push_subscriptions (user_id);
";
