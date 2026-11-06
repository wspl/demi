//! From the control schema whose port forward is its route: a conversation
//! records when it was archived, which also says whether it is. No release
//! recorded that time, so one already archived takes its last activity.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_PORT_FORWARD_ROUTES);

const CONTROL_FROM_0_1_5979_PORT_FORWARD_ROUTES: &str = "
ALTER TABLE conversations ADD COLUMN archived_at INTEGER;
UPDATE conversations SET archived_at = updated_at WHERE archived = 1;
DROP INDEX conversations_sidebar;
ALTER TABLE conversations DROP COLUMN archived;
CREATE INDEX conversations_sidebar ON conversations (user_id, archived_at, pinned, sort_order);
";
