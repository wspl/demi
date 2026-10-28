//! From the form a child's directory was kept in with where the conversation
//! worked when it was set, which a move of the conversation overrode, before
//! a child's placement was its own: a child keeps the Host and the directory
//! it was last given.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONVERSATION_FROM_DIRECTORIES_FOLLOWING_MOVES);

const CONVERSATION_FROM_DIRECTORIES_FOLLOWING_MOVES: &str = "
ALTER TABLE nodes RENAME COLUMN cwd_device TO placement_device;
ALTER TABLE nodes RENAME COLUMN cwd TO placement_directory;
ALTER TABLE nodes DROP COLUMN cwd_under;";
