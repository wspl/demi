//! From 0.1.18's control schema: the deployment keeps its namespace at the
//! preview domain, which a deployment migrated from 0.1.18 registers at its
//! next start.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_18);

const CONTROL_FROM_0_1_18: &str = "
CREATE TABLE preview_namespace (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  namespace  TEXT NOT NULL,
  secret     BLOB NOT NULL,
  expires_at INTEGER NOT NULL,
  origins    TEXT NOT NULL
) STRICT;
";
