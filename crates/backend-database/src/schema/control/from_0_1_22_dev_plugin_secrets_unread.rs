//! From the control schema with the plugins' secrets, which a server keeps,
//! to the one where a request has a read state: a request that waits has not
//! been read yet.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_PLUGIN_SECRETS_UNREAD);

const CONTROL_FROM_PLUGIN_SECRETS_UNREAD: &str =
    "ALTER TABLE permission_requests ADD COLUMN read INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1));";
