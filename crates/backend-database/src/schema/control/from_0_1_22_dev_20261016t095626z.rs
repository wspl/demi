//! From 0.1.22-dev.20261016T095626Z's control schema, which a server keeps
//! data of, to 0.1.22's: each account has the size of its Cloud
//! disks, 16 GB each until an administrator changes it.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_22_DEV_20261016T095626Z);

const CONTROL_FROM_0_1_22_DEV_20261016T095626Z: &str = "
ALTER TABLE users ADD COLUMN cloud_system_gb INTEGER NOT NULL DEFAULT 16 CHECK (cloud_system_gb BETWEEN 1 AND 1024);
ALTER TABLE users ADD COLUMN cloud_home_gb INTEGER NOT NULL DEFAULT 16 CHECK (cloud_home_gb BETWEEN 1 AND 1024);
";
