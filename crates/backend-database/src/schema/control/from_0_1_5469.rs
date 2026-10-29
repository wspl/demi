//! From 0.1.5469's control schema, a development build's that a server
//! keeps data of, to 0.1.5541's:
//!
//! - An entry asks for a prompt cache lifetime, which an Anthropic entry's
//!   requests asked an hour for until then, and a root may compact while
//!   idle.
//! - The server has settings of its own, which name the channel its updates
//!   come from.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5469);

const CONTROL_FROM_0_1_5469: &str = "
ALTER TABLE providers ADD COLUMN cache_ttl TEXT;
UPDATE providers SET cache_ttl = '1h' WHERE provider_type = 'anthropic';
CREATE TABLE idle_compactions (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  due_at          INTEGER NOT NULL,
  lifetime_ms     INTEGER NOT NULL CHECK (lifetime_ms > 0)
) STRICT;
CREATE TABLE server_settings (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  upgrade_channel TEXT NOT NULL CHECK (upgrade_channel IN ('beta', 'canary'))
) STRICT;
";
