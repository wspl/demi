//! From 0.1.5950's control schema: a video call keeps the SHA-256 of the
//! video it made, by which a change of that video finds the job that made
//! it.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5950);

const CONTROL_FROM_0_1_5950: &str = "
ALTER TABLE usage_ledger ADD COLUMN media_digest TEXT;
CREATE INDEX usage_ledger_media_digest ON usage_ledger (media_digest) WHERE media_digest IS NOT NULL;
";
