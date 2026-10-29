//! From 0.1.36's control schema, which 0.1.33 to 0.1.35 shipped too, to
//! 0.1.37's.
//!
//! - A saved message is edited in the composer and written back in its place,
//!   so the moves of one into the draft, whose answers `saved_edits` kept, go
//!   with their answers.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_36);

const CONTROL_FROM_0_1_36: &str = "
DROP TABLE saved_edits;
";
