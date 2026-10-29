//! From 0.1.5708's conversation schema: blocks are written in transcript
//! block format 17, which keeps when each part of a turn started and ended.
//! No row changes: a record names its format, and format 16 is still read.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql("");
