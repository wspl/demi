//! From the control schema that keeps the turns a shutdown interrupted to the one
//! in which a conversation keeps the hosts it may reach and a request may ask for
//! one (`crates/backend-database/src/conversation_index.rs`), which replace the
//! Manage Devices category.
//!
//! - Every attached host seeds its conversation's list: a host attached by name
//!   took Manage Devices to attach, and one attached with a directory is a primary
//!   target the conversation left, which the list holds anyway.
//! - The grants of Manage Devices go, and so do the undecided requests that asked
//!   for it, which no command asks for any more. A decided one stays until its
//!   message is delivered.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_INTERRUPTED_TURNS);

const CONTROL_FROM_INTERRUPTED_TURNS: &str = "
ALTER TABLE permission_requests ADD COLUMN host_device_id TEXT
    CHECK (host_device_id IS NULL OR kind = 'permission');
ALTER TABLE permission_requests ADD COLUMN pairing_code TEXT
    CHECK (pairing_code IS NULL OR (kind = 'permission' AND host_device_id IS NULL));
CREATE TABLE reachable_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  added_at        INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
CREATE INDEX reachable_hosts_of_device ON reachable_hosts (device_id);
CREATE TABLE pairing_passes (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  code            TEXT NOT NULL,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, code)
) STRICT;
INSERT INTO reachable_hosts (conversation_id, device_id, added_at)
  SELECT conversation_id, device_id, attached_at FROM conversation_hosts;
DELETE FROM permission_grants WHERE category = 'host.devices';
DELETE FROM permission_requests
  WHERE kind = 'permission' AND decision IS NULL
    AND EXISTS (SELECT 1 FROM json_each(categories) WHERE value = 'host.devices');
";
