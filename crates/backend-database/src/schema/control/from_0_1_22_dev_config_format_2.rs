//! From the control schema whose API-key entries sealed configuration format 2 to
//! the one with account rotation (`crates/provider-registry/src/rotation.rs`).
//!
//! - Every entry keeps its manual rotation.
//! - Each entry's accounts keep the order they were listed in, by their ids.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_CONFIG_FORMAT_2);

const CONTROL_FROM_CONFIG_FORMAT_2: &str = "
ALTER TABLE providers ADD COLUMN rotation TEXT NOT NULL DEFAULT 'manual'
    CHECK (rotation IN ('manual', 'in_order', 'balanced')
      AND (credential_kind = 'subscription' OR rotation = 'manual'));
ALTER TABLE provider_credentials ADD COLUMN position INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0);
UPDATE provider_credentials SET position = (
  SELECT count(*) FROM provider_credentials AS earlier
  WHERE earlier.provider_id = provider_credentials.provider_id AND earlier.id < provider_credentials.id
);
";
