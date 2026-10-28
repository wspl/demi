//! From the form in which a user held one Cloud, before the user's record
//! named their own Cloud and a user could hold others: each user's Cloud is
//! their own.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_ONE_CLOUD);

const CONTROL_FROM_ONE_CLOUD: &str = "
ALTER TABLE users ADD COLUMN cloud_device_id TEXT;
UPDATE users SET cloud_device_id =
  (SELECT id FROM devices WHERE devices.user_id = users.id AND kind = 'managed' AND machine = 'cloud');
DROP INDEX devices_one_machine;
CREATE UNIQUE INDEX devices_one_claude_code ON devices (user_id) WHERE machine = 'claude_code';";
