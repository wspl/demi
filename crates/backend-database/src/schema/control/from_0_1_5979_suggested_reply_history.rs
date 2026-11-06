//! From the control schema whose suggested replies answers stay: a reply is its
//! words, and the new topics go.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_SUGGESTED_REPLY_HISTORY);

const CONTROL_FROM_0_1_5979_SUGGESTED_REPLY_HISTORY: &str = "
UPDATE conversation_suggested_replies SET replies = (
  SELECT json_group_array(json_extract(reply.value, '$.text') ORDER BY reply.key)
  FROM json_each(conversation_suggested_replies.replies) AS reply
  WHERE json_extract(reply.value, '$.kind') = 'answer'
);
";
