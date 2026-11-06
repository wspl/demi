//! From the control schema whose permission requests name a device: a
//! device is called a device. The attached devices, their revision and the
//! workflows kept per device take the word, and so do the preferences' last
//! project device and a Fork's attached devices.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_DEVICE_REQUESTS);

const CONTROL_FROM_0_1_5979_DEVICE_REQUESTS: &str = "
ALTER TABLE host_workflows RENAME TO device_workflows;
ALTER TABLE conversations RENAME COLUMN hosts_revision TO devices_revision;
DROP INDEX conversation_hosts_device;
ALTER TABLE conversation_hosts RENAME TO conversation_devices;
CREATE INDEX conversation_devices_device ON conversation_devices (device_id);
UPDATE user_preferences
  SET preferences = json_set(
    json_remove(preferences, '$.lastProjectHost'),
    '$.lastProjectDevice',
    json(json_extract(preferences, '$.lastProjectHost'))
  )
  WHERE json_type(preferences, '$.lastProjectHost') IS NOT NULL;
UPDATE conversation_fork_operations
  SET metadata = json_set(
    json_remove(metadata, '$.attachedHosts'),
    '$.attachedDevices',
    json(json_extract(metadata, '$.attachedHosts'))
  )
  WHERE json_type(metadata, '$.attachedHosts') IS NOT NULL;
";
