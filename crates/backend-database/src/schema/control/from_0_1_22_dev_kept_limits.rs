//! From the control schema whose subagent limits keep their numbers to the
//! one with the plugins' secrets and OAuth clients: two new tables, empty.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_KEPT_LIMITS);

const CONTROL_FROM_KEPT_LIMITS: &str = "
CREATE TABLE plugin_secrets (
  user_id  TEXT NOT NULL REFERENCES users (id),
  plugin   TEXT NOT NULL,
  name     TEXT NOT NULL,
  sealed   BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  PRIMARY KEY (user_id, plugin, name)
) STRICT, WITHOUT ROWID;
CREATE TABLE plugin_oauth_clients (
  plugin       TEXT NOT NULL,
  issuer       TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  client_id    TEXT NOT NULL,
  PRIMARY KEY (plugin, issuer)
) STRICT, WITHOUT ROWID;
";
