package storage

// The numbered schema changes, as the TypeScript backend defines them
// (packages/backend/src/storage/migrations.ts): the same ids, names and SQL,
// so either backend opens the other's data directory. TEXT ids, TEXT ISO
// timestamps, INTEGER booleans.

// ControlMigrations build control.sqlite, the control plane.
var ControlMigrations = []Migration{
	{
		ID:   1,
		Name: "init",
		SQL: `
CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
  nickname      TEXT NOT NULL DEFAULT '',
  password_hash TEXT NOT NULL,
  role          TEXT NOT NULL CHECK (role IN ('master', 'admin', 'user')),
  created_at    TEXT NOT NULL
);

CREATE TABLE user_preferences (
  user_id TEXT PRIMARY KEY REFERENCES users(id),
  preferences_json TEXT NOT NULL
);

CREATE TABLE email_challenges (
  user_id TEXT PRIMARY KEY REFERENCES users(id),
  id TEXT NOT NULL UNIQUE,
  email TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  code_hash TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  sent_at INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE web_sessions (
  token_hash TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id),
  expires_at TEXT NOT NULL
);

CREATE TABLE devices (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users(id),
  kind                  TEXT NOT NULL CHECK (kind IN ('user', 'managed')),
  name                  TEXT NOT NULL,
  platform              TEXT NOT NULL,
  token_hash            TEXT NOT NULL,
  claimed_at            TEXT NOT NULL,
  last_seen_at          TEXT
);

CREATE UNIQUE INDEX idx_managed_user ON devices(user_id) WHERE kind = 'managed';

CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id),
  device_id  TEXT NOT NULL REFERENCES devices(id),
  path       TEXT NOT NULL,
  name       TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE conversations (
  id            TEXT PRIMARY KEY,
  user_id       TEXT NOT NULL REFERENCES users(id),
  title         TEXT NOT NULL,
  archived      INTEGER NOT NULL DEFAULT 0,
  pinned        INTEGER NOT NULL DEFAULT 0,
  sort_order    INTEGER NOT NULL DEFAULT 0,
  read_revision INTEGER NOT NULL DEFAULT 0,
  target_json   TEXT NOT NULL DEFAULT '{"kind":"cloud"}',
  last_switch_json TEXT,
      cloud_reset_id TEXT,
  context_version INTEGER NOT NULL DEFAULT 0,
  provider_id TEXT,
  model_id      TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE INDEX idx_conversations_user ON conversations(user_id, archived, updated_at);

CREATE TABLE conversation_fork_operations (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  source_id TEXT NOT NULL,
  block_id TEXT NOT NULL,
  metadata_json TEXT NOT NULL
);

CREATE TABLE managed_operations (
  device_id TEXT NOT NULL REFERENCES devices(id),
  operation_id TEXT NOT NULL,
  operation_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (device_id, operation_id)
);

CREATE TABLE conversation_hosts (
  conversation_id TEXT NOT NULL REFERENCES conversations(id),
  device_id       TEXT NOT NULL REFERENCES devices(id),
  name            TEXT NOT NULL,
  cwd             TEXT,
  attached_at     TEXT NOT NULL,
  PRIMARY KEY (conversation_id, device_id),
  UNIQUE (conversation_id, name)
);
CREATE INDEX idx_conversation_hosts_device ON conversation_hosts(device_id);

CREATE TABLE providers (
  id            TEXT PRIMARY KEY,
  owner_user_id TEXT REFERENCES users(id),
  provider_type TEXT NOT NULL,
  credential_kind TEXT NOT NULL CHECK (credential_kind IN ('api_key', 'subscription')),
  label         TEXT NOT NULL,
  config        TEXT NOT NULL,
  created_at    TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_providers_subscription_scope ON providers(COALESCE(owner_user_id, ''), provider_type)
  WHERE credential_kind = 'subscription';

CREATE TABLE usage_ledger (
  id              TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users(id),
  conversation_id TEXT NOT NULL,
  provider_id   TEXT NOT NULL,
  model_id        TEXT NOT NULL,
  input_tokens    INTEGER NOT NULL,
  output_tokens   INTEGER NOT NULL,
  cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_usage_ledger_user ON usage_ledger(user_id, created_at);

CREATE TABLE attachments (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id),
  media_type TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  sha256     TEXT NOT NULL,
  created_at TEXT NOT NULL
);
`,
	},
	{
		ID:   2,
		Name: "model_catalogs",
		SQL: `
CREATE TABLE model_catalogs (
  provider_id TEXT PRIMARY KEY REFERENCES providers(id) ON DELETE CASCADE,
  record_json TEXT NOT NULL
);
`,
	},
	{
		ID:   3,
		Name: "exposes",
		SQL: `
CREATE TABLE exposes (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id),
  device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  address    TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);
CREATE INDEX idx_exposes_expiry ON exposes(expires_at);
`,
	},
	{
		// Where a title came from (product.md § Conversation titles). A row that
		// names no origin carries a settled title: a Fork's, or one a caller chose.
		ID:   4,
		Name: "conversation_title_origin",
		SQL: `
ALTER TABLE conversations ADD COLUMN title_origin TEXT NOT NULL DEFAULT 'user'
  CHECK (title_origin IN ('placeholder', 'message', 'generated', 'user'));
UPDATE conversations SET title_origin = 'placeholder' WHERE title = 'New conversation';
`,
	},
	{
		// How many messages the user has sent, and how many the last generated
		// title had seen (product.md § Conversation titles). Equal on every
		// existing row: its title stands until the next message.
		ID:   5,
		Name: "conversation_title_currency",
		SQL: `
ALTER TABLE conversations ADD COLUMN user_messages INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversations ADD COLUMN titled_messages INTEGER NOT NULL DEFAULT 0;
`,
	},
	{
		// Every provider has an owner (product.md § Instance mode): a shared
		// instance's entries, once ownerless, are the master's. SQLite cannot add
		// NOT NULL to a column in place; the control service writes no other value.
		ID:   6,
		Name: "providers_belong_to_users",
		SQL: `
UPDATE providers SET owner_user_id = (SELECT id FROM users WHERE role = 'master')
  WHERE owner_user_id IS NULL;
DROP INDEX idx_providers_subscription_scope;
CREATE UNIQUE INDEX idx_providers_subscription_owner ON providers(owner_user_id, provider_type)
  WHERE credential_kind = 'subscription';
`,
	},
	{
		// A subscription entry's accounts (providers-and-vault.md § Credential
		// vault): the secret document is encrypted like `providers.config`, every
		// secret write advances `version`, and `quota` is the account's usage
		// snapshot. The entry names the account it infers with.
		ID:   7,
		Name: "provider_credentials",
		SQL: `
CREATE TABLE provider_credentials (
  provider_id  TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  id           TEXT NOT NULL,
  identity_key TEXT,
  label        TEXT NOT NULL,
  detail       TEXT,
  source       TEXT,
  secret       TEXT NOT NULL,
  version      INTEGER NOT NULL,
  quota        TEXT,
  updated_at   TEXT NOT NULL,
  PRIMARY KEY (provider_id, id)
);
CREATE UNIQUE INDEX idx_provider_credentials_identity ON provider_credentials(provider_id, identity_key)
  WHERE identity_key IS NOT NULL;
ALTER TABLE providers ADD COLUMN active_credential_id TEXT;
`,
	},
	{
		// The work panel's saved state (web-api.md § Work panel state): one JSON
		// document per conversation, which only the page interprets.
		ID:   8,
		Name: "conversation_panels",
		SQL: `
CREATE TABLE conversation_panels (
  conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
  document_json   TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
`,
	},
}

// ConversationMigrations build every conversations/<id>.sqlite: the agent
// journal and node-scoped command state. Files live on devices.
var ConversationMigrations = []Migration{
	{
		ID:   1,
		Name: "init",
		SQL: `
CREATE TABLE nodes (
  id            TEXT PRIMARY KEY,
  parent_id     TEXT REFERENCES nodes(id) ON DELETE CASCADE,
  description   TEXT NOT NULL,
  profile_name  TEXT,
  metadata_json TEXT,
  spawned_at    INTEGER NOT NULL,
  can_spawn     INTEGER NOT NULL,
  closed_phase  TEXT CHECK (closed_phase IN ('completed', 'aborted', 'error')),
  closed_at     INTEGER,
  result        TEXT,
  failure       TEXT,
  delivered     INTEGER NOT NULL DEFAULT 0,
  state_json    TEXT NOT NULL,
  command_revision INTEGER NOT NULL DEFAULT 0 CHECK (command_revision >= 0),
  output_revision INTEGER NOT NULL DEFAULT 0,
  block_count   INTEGER NOT NULL
);
CREATE INDEX idx_nodes_parent ON nodes(parent_id, spawned_at);

CREATE TABLE blocks (
  node_id    TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  idx        INTEGER NOT NULL,
  block_json TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
);

CREATE TABLE command_snapshots (
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK (revision >= 0),
  values_json TEXT NOT NULL,
  PRIMARY KEY (node_id, revision)
);

CREATE TABLE session_boundaries (
  node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  block_id TEXT NOT NULL,
  edge TEXT NOT NULL CHECK (edge IN ('before_user', 'after_assistant', 'after_block')),
  command_revision INTEGER NOT NULL,
  PRIMARY KEY (node_id, block_id, edge),
  FOREIGN KEY (node_id, command_revision) REFERENCES command_snapshots(node_id, revision)
);

CREATE TABLE host_store (
  scope      TEXT NOT NULL,
  key        TEXT NOT NULL,
  value_json TEXT NOT NULL,
  PRIMARY KEY (scope, key)
);

`,
	},
}
