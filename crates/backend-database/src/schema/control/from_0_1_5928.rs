//! From 0.1.5928's control schema, which 0.1.5933, 0.1.5939 and 0.1.5941
//! shipped too: an entry keeps the models its vendor refused to its key or
//! routed to another model, which its list no longer offers
//! (`crates/provider-registry/src/catalog.rs`).

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5928);

const CONTROL_FROM_0_1_5928: &str = "
CREATE TABLE retired_models (
  provider_id TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  key_digest  TEXT NOT NULL,
  model_id    TEXT NOT NULL,
  answered_by TEXT,
  retired_at  INTEGER NOT NULL,
  PRIMARY KEY (provider_id, key_digest, model_id)
) STRICT;
";
