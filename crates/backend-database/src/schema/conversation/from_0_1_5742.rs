//! From 0.1.5742's conversation schema: blocks are written in transcript
//! block format 18, whose entries name their owner and whose redacted
//! reasoning is an `opaque` block. A record names its format, and format 17
//! is still read as 18; the step renames the type the rows' column states.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(
    "UPDATE blocks SET type = 'opaque' WHERE type = 'redacted_thinking';
     UPDATE share_blocks SET type = 'opaque' WHERE type = 'redacted_thinking';",
);
