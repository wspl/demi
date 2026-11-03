//! Gives each usage row the integration's message its work descends from,
//! and writes an integration's declared context as the mode it now names:
//! `optional` for a source, `none` for none.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_USAGE_WITHOUT_MESSAGES);

const CONTROL_FROM_USAGE_WITHOUT_MESSAGES: &str = r"
ALTER TABLE usage_ledger ADD COLUMN message_id TEXT;
UPDATE integrations
  SET declaration = json_set(declaration, '$.context',
    CASE json_extract(declaration, '$.context') WHEN 1 THEN 'optional' ELSE 'none' END)
  WHERE declaration IS NOT NULL;
";
