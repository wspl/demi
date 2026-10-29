//! From 0.1.25's control schema to 0.1.27's.
//!
//! - An entry whose provider makes images has an Image Generation switch, on for
//!   every entry stored before
//!   (`crates/backend-user-shard/src/provider_settings.rs`).
//! - A device keeps why it will not start its runner by itself, which no device
//!   stored before has said (`crates/runner/src/autostart.rs`).

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_25);

const CONTROL_FROM_0_1_25: &str = "
ALTER TABLE providers ADD COLUMN image_generation INTEGER NOT NULL DEFAULT 1 CHECK (image_generation IN (0, 1));
ALTER TABLE devices ADD COLUMN autostart_unavailable TEXT;
";
