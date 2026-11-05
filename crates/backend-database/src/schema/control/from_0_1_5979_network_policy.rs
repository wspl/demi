//! From the control schema whose network policy decides what a Cloud
//! reaches: a port forward is its route, and loses its name. The table is
//! rebuilt, since its unique constraint names the column.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_NETWORK_POLICY);

const CONTROL_FROM_0_1_5979_NETWORK_POLICY: &str = "
CREATE TABLE port_forwards_next (
  id                TEXT PRIMARY KEY,
  user_id           TEXT NOT NULL REFERENCES users (id),
  listen_device_id  TEXT NOT NULL REFERENCES devices (id),
  listen_port       INTEGER NOT NULL CHECK (listen_port BETWEEN 1 AND 65535),
  connect_device_id TEXT REFERENCES devices (id),
  connect_address   TEXT NOT NULL,
  enabled           INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  created_at        INTEGER NOT NULL
) STRICT;
INSERT INTO port_forwards_next (id, user_id, listen_device_id, listen_port, connect_device_id, connect_address, enabled, created_at)
  SELECT id, user_id, listen_device_id, listen_port, connect_device_id, connect_address, enabled, created_at FROM port_forwards;
DROP TABLE port_forwards;
ALTER TABLE port_forwards_next RENAME TO port_forwards;
CREATE INDEX port_forwards_of_user ON port_forwards (user_id, created_at);
";
