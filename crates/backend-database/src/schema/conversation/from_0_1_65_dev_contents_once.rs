//! From the form each content was kept once in, before a command's row kept
//! what its result told of its media.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONVERSATION_FROM_CONTENTS_ONCE);

const CONVERSATION_FROM_CONTENTS_ONCE: &str = "ALTER TABLE command_outputs ADD COLUMN media TEXT;";
