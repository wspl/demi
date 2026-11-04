//! Gives integrations' Clouds the records of their commands, none yet, and
//! each integration the `titles` capability, on as every new one has it.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_NO_CLOUD_COMMANDS);

const CONTROL_FROM_NO_CLOUD_COMMANDS: &str = r"
CREATE TABLE cloud_commands (
  id             TEXT PRIMARY KEY,
  device_id      TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  started_at     INTEGER NOT NULL,
  job_id         TEXT,
  ended_at       INTEGER,
  exit_code      INTEGER,
  failure        TEXT,
  blob           TEXT,
  missing_bytes  INTEGER,
  missing_reason TEXT,
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL))
) STRICT;
CREATE INDEX cloud_commands_device ON cloud_commands (device_id);
UPDATE integrations SET capabilities = json_set(capabilities, '$.titles', json('true'))
  WHERE json_type(capabilities, '$.titles') IS NULL;
";
