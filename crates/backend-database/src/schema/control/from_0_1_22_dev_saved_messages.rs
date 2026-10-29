//! From the control schema with the messages saved for later to the one with
//! administered accounts.
//!
//! - An account is enabled and has chosen its own password, as every account that
//!   exists did, and was last active when its latest session was.
//! - A session was not made by an administrator's code.
//! - Each user's one handoff code becomes their handoff code of its kind, and no
//!   sign-in code exists.
//!
//! The codes' table is rebuilt, since SQLite changes a primary key only so.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_SAVED_MESSAGES);

const CONTROL_FROM_SAVED_MESSAGES: &str = "
ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1));
ALTER TABLE users ADD COLUMN chooses_password INTEGER NOT NULL DEFAULT 0 CHECK (chooses_password IN (0, 1));
ALTER TABLE users ADD COLUMN last_active_at INTEGER;
UPDATE users SET last_active_at = (SELECT max(last_active_at) FROM web_sessions WHERE user_id = users.id);
ALTER TABLE web_sessions ADD COLUMN issued_by TEXT REFERENCES users (id);
CREATE TABLE handoff_codes_new (
  user_id    TEXT NOT NULL REFERENCES users (id),
  kind       TEXT NOT NULL CHECK (kind IN ('handoff', 'sign_in')),
  issued_by  TEXT REFERENCES users (id),
  id         TEXT NOT NULL UNIQUE,
  code_hash  TEXT NOT NULL UNIQUE,
  to_path    TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  spent      INTEGER NOT NULL CHECK (spent IN (0, 1)),
  PRIMARY KEY (user_id, kind),
  CHECK ((kind = 'sign_in') = (issued_by IS NOT NULL))
) STRICT;
INSERT INTO handoff_codes_new (user_id, kind, id, code_hash, to_path, expires_at, spent)
  SELECT user_id, 'handoff', id, code_hash, to_path, expires_at, spent FROM handoff_codes;
DROP TABLE handoff_codes;
ALTER TABLE handoff_codes_new RENAME TO handoff_codes;
";
