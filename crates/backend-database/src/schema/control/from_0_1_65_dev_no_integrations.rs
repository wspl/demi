//! From the form before integrations: their tables.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_NO_INTEGRATIONS);

const CONTROL_FROM_NO_INTEGRATIONS: &str = r"
CREATE TABLE integrations (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  -- The workspace its conversations are in, which no page lists.
  workspace_id TEXT NOT NULL UNIQUE REFERENCES workspaces (id),
  name         TEXT NOT NULL CHECK (name <> ''),
  -- The SHA-256 of its token, which authenticates its requests.
  token_hash   TEXT NOT NULL UNIQUE,
  -- What its agents may do, IntegrationCapabilities as JSON.
  capabilities TEXT NOT NULL,
  created_at   INTEGER NOT NULL
) STRICT;
CREATE INDEX integrations_by_user ON integrations (user_id);
CREATE TABLE integration_conversations (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  integration_id  TEXT NOT NULL REFERENCES integrations (id),
  -- The string the integration finds it by, unique within the integration;
  -- none for one it keeps no key of.
  external_key    TEXT,
  -- The workflow it was created with, YAML in the format of a project's
  -- workflow file; none for no subagent profiles.
  workflow        TEXT
) STRICT;
CREATE UNIQUE INDEX integration_conversations_by_key
  ON integration_conversations (integration_id, external_key) WHERE external_key IS NOT NULL;
CREATE TABLE integration_clouds (
  device_id      TEXT PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
  integration_id TEXT NOT NULL REFERENCES integrations (id),
  external_key   TEXT
) STRICT;
CREATE UNIQUE INDEX integration_clouds_by_key
  ON integration_clouds (integration_id, external_key) WHERE external_key IS NOT NULL;
";
