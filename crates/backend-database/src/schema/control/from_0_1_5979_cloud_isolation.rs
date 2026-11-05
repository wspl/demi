//! From the control schema of the Cloud isolation, which no release
//! shipped: Private Network Access becomes the network policy, which also
//! decides whether Clouds reach the internet. Each Cloud reaches what it
//! reached before: Off becomes Internet Only, Entire Private Network becomes
//! Internet and Private Network, and Only These Addresses becomes Custom with
//! the internet and the same addresses. The addresses are kept as Custom's
//! list whatever the choice, as they were kept before.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_CLOUD_ISOLATION);

const CONTROL_FROM_0_1_5979_CLOUD_ISOLATION: &str = "
ALTER TABLE server_settings ADD COLUMN network_policy TEXT NOT NULL DEFAULT 'internetOnly'
  CHECK (network_policy IN ('noNetwork', 'developerServices', 'internetOnly', 'internetAndPrivateNetwork', 'custom'));
ALTER TABLE server_settings ADD COLUMN custom_internet INTEGER NOT NULL DEFAULT 1 CHECK (custom_internet IN (0, 1));
ALTER TABLE server_settings ADD COLUMN custom_private_network INTEGER NOT NULL DEFAULT 0
  CHECK (custom_private_network IN (0, 1));
ALTER TABLE server_settings ADD COLUMN custom_developer_services INTEGER NOT NULL DEFAULT 0
  CHECK (custom_developer_services IN (0, 1));
ALTER TABLE server_settings ADD COLUMN custom_allowed TEXT NOT NULL DEFAULT '[]';
UPDATE server_settings SET
  network_policy = CASE private_network
    WHEN 'off' THEN 'internetOnly'
    WHEN 'entire' THEN 'internetAndPrivateNetwork'
    ELSE 'custom'
  END,
  custom_allowed = private_destinations;
ALTER TABLE server_settings DROP COLUMN private_network;
ALTER TABLE server_settings DROP COLUMN private_destinations;
";
