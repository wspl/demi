//! From 0.1.5565's control schema to 0.1.5586's:
//!
//! - A push subscription says whether it stays quiet while the user is on
//!   another device, which every one stored before does, as the switch is on
//!   until turned off.
//! - A conversation keeps the status of its notifications card, none before.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5565);

const CONTROL_FROM_0_1_5565: &str = "
ALTER TABLE push_subscriptions ADD COLUMN quiet_while_elsewhere INTEGER NOT NULL DEFAULT 1
  CHECK (quiet_while_elsewhere IN (0, 1));
ALTER TABLE conversations ADD COLUMN notifications_card TEXT
  CHECK (notifications_card IN ('shown', 'dismissed'));
";
