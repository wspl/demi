//! Gives integrations an init script and a shared folder's path, and
//! their Clouds the state of their script's run.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_UNINITIALIZED_INTEGRATIONS);

const CONTROL_FROM_UNINITIALIZED_INTEGRATIONS: &str = r"
ALTER TABLE integrations ADD COLUMN init_script TEXT;
ALTER TABLE integrations ADD COLUMN shared_path TEXT NOT NULL DEFAULT '~/shared';
ALTER TABLE integration_clouds ADD COLUMN init TEXT NOT NULL DEFAULT 'pending'
  CHECK (init IN ('pending', 'running', 'done', 'failed', 'unscripted'));
ALTER TABLE integration_clouds ADD COLUMN init_exit_code INTEGER;
ALTER TABLE integration_clouds ADD COLUMN init_output TEXT;
ALTER TABLE usage_ledger ADD COLUMN device_id TEXT;
ALTER TABLE usage_ledger ADD COLUMN integration_id TEXT;
CREATE INDEX usage_ledger_integration ON usage_ledger (integration_id, ended_at)
  WHERE integration_id IS NOT NULL;
";
