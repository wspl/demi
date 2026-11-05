//! From the control schema of subscriptions that connect through a device,
//! which no release shipped: the server settings say how Clouds are
//! isolated. A server that migrates ran every Cloud on gVisor, and stays on
//! it; a new one starts on Simple Isolation.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_SUBSCRIPTION_DEVICES);

const CONTROL_FROM_0_1_5979_SUBSCRIPTION_DEVICES: &str = "
ALTER TABLE server_settings ADD COLUMN isolation TEXT NOT NULL DEFAULT 'simple' CHECK (isolation IN ('simple', 'gvisor'));
INSERT INTO server_settings (id, isolation) VALUES (1, 'gvisor')
  ON CONFLICT (id) DO UPDATE SET isolation = excluded.isolation;
";
