//! Gives each integration's conversation its identity and the roots of its
//! integration's declaration it has: none of either, Demi's identity and
//! every root, as before.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_CONVERSATIONS_WITHOUT_IDENTITIES);

const CONTROL_FROM_CONVERSATIONS_WITHOUT_IDENTITIES: &str = r"
ALTER TABLE integration_conversations ADD COLUMN identity TEXT;
ALTER TABLE integration_conversations ADD COLUMN commands TEXT;
";
