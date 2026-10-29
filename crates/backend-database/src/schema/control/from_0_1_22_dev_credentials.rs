//! From the control schema with the users' credentials to the one with
//! passkeys (`crates/backend/src/config/secret.rs`): two new tables,
//! empty.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_CREDENTIALS);

const CONTROL_FROM_CREDENTIALS: &str = "
CREATE TABLE passkeys (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  credential   TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  last_used_at INTEGER
) STRICT;
CREATE INDEX passkeys_of_user ON passkeys (user_id, created_at);
CREATE TABLE passkey_challenges (
  id         TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('sign_in', 'registration')),
  user_id    TEXT REFERENCES users (id),
  state      TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  CHECK ((kind = 'registration') = (user_id IS NOT NULL))
) STRICT;
CREATE INDEX passkey_challenges_expiry ON passkey_challenges (expires_at);
";
