//! Gives each integration's conversation the browser profile it names: none,
//! its own profile, as before.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_CONVERSATIONS_WITHOUT_BROWSER_PROFILES);

const CONTROL_FROM_CONVERSATIONS_WITHOUT_BROWSER_PROFILES: &str = r"
ALTER TABLE integration_conversations ADD COLUMN browser_profile TEXT;
";
