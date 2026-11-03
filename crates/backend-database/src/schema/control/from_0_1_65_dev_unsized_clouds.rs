//! Gives each integration its Clouds' sizing and idle window, the server's,
//! and each of its Clouds the always-on flag, off.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_UNSIZED_CLOUDS);

const CONTROL_FROM_UNSIZED_CLOUDS: &str = r"
ALTER TABLE integrations ADD COLUMN cloud_sizing TEXT NOT NULL DEFAULT '{}';
ALTER TABLE integrations ADD COLUMN cloud_idle_minutes INTEGER CHECK (cloud_idle_minutes > 0);
ALTER TABLE integration_clouds ADD COLUMN always_on INTEGER NOT NULL DEFAULT 0 CHECK (always_on IN (0, 1));
";
