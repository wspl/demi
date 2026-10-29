//! From the control schema with passkeys to the one with the signed-in browsers
//! and handoff codes.
//!
//! - Each session gets an id, and the browser it signed in from, which no session
//!   before kept, reads `Web Browser`.
//! - When it signed in and was last active, which none kept either, read its last
//!   renewal, 30 days before its expiry.
//! - No session was verified.
//! - The handoff codes start empty.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_PASSKEYS);

const CONTROL_FROM_PASSKEYS: &str = "
CREATE TABLE web_sessions_new (
  token_hash     TEXT PRIMARY KEY,
  id             TEXT NOT NULL UNIQUE,
  user_id        TEXT NOT NULL REFERENCES users (id),
  expires_at     INTEGER NOT NULL,
  name           TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  last_active_at INTEGER NOT NULL,
  verified_at    INTEGER,
  handoff_id     TEXT
) STRICT;
INSERT INTO web_sessions_new (token_hash, id, user_id, expires_at, name, created_at, last_active_at)
  SELECT token_hash, lower(hex(randomblob(16))), user_id, expires_at, 'Web Browser',
         expires_at - 2592000000, expires_at - 2592000000
  FROM web_sessions;
DROP TABLE web_sessions;
ALTER TABLE web_sessions_new RENAME TO web_sessions;
CREATE INDEX web_sessions_expiry ON web_sessions (expires_at);
CREATE INDEX web_sessions_of_user ON web_sessions (user_id, created_at);
CREATE TABLE handoff_codes (
  user_id    TEXT PRIMARY KEY REFERENCES users (id),
  id         TEXT NOT NULL UNIQUE,
  code_hash  TEXT NOT NULL UNIQUE,
  to_path    TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  spent      INTEGER NOT NULL CHECK (spent IN (0, 1))
) STRICT;
";
