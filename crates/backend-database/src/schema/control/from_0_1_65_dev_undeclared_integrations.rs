//! From the form before an integration declared a plugin: none has
//! declared one.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_UNDECLARED_INTEGRATIONS);

const CONTROL_FROM_UNDECLARED_INTEGRATIONS: &str = "ALTER TABLE integrations ADD COLUMN declaration TEXT;";
