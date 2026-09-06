import type { SqlDatabase } from './database'

/**
 * Numbered migrations, applied in order inside one transaction each: one set
 * for `control.sqlite` (the control plane) and one for every
 * `conversations/<id>.sqlite` (the data plane). TEXT ids, TEXT ISO
 * timestamps, INTEGER booleans.
 *
 * (The design record called for `.sql` files; they live as tagged constants in
 * this one module instead so the published bundle needs no file-tree lookup —
 * same numbering, same content discipline.)
 */
export interface Migration {
  id: number
  name: string
  sql: string
}

export const CONTROL_MIGRATIONS: Migration[] = [
  {
    id: 1,
    name: 'init',
    sql: `
CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  username      TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role          TEXT NOT NULL CHECK (role IN ('master', 'admin', 'user')),
  created_at    TEXT NOT NULL
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
  owner_conversation_id TEXT,
  owner_workspace_id    TEXT,
  claimed_at            TEXT NOT NULL,
  last_seen_at          TEXT
);

CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id),
  device_id  TEXT NOT NULL REFERENCES devices(id),
  path       TEXT NOT NULL,
  name       TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE conversations (
  id            TEXT PRIMARY KEY,
  user_id       TEXT NOT NULL REFERENCES users(id),
  title         TEXT NOT NULL,
  archived      INTEGER NOT NULL DEFAULT 0,
  workspace_id  TEXT REFERENCES workspaces(id),
  host_device_id TEXT REFERENCES devices(id),
  last_switch_json TEXT,
  context_version INTEGER NOT NULL DEFAULT 0,
  provider_id TEXT,
  model_id      TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE INDEX idx_conversations_user ON conversations(user_id, archived, updated_at);

CREATE TABLE conversation_upgrades (
  conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
  state TEXT NOT NULL CHECK (state IN ('prepared', 'committed'))
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
]

/**
 * Per-conversation database: the session tree — one node row per agent (the
 * root, whose id is the conversation's, and every subagent under it) with
 * its state row, and each node's transcript as one row per block
 * (`subagent.md` § Persistence) — that conversation's host_store scope, and
 * the hostless filesystem's tree (`storage.md` § The hostless filesystem and
 * the home image) — bytes in the blob store by sha256, emptied once the
 * conversation has a home image. Media bytes never appear in block_json —
 * they live in the blob store as content-addressed refs.
 */
export const CONVERSATION_MIGRATIONS: Migration[] = [
  {
    id: 1,
    name: 'init',
    sql: `
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
  block_count   INTEGER NOT NULL
);
CREATE INDEX idx_nodes_parent ON nodes(parent_id, spawned_at);

CREATE TABLE blocks (
  node_id    TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  idx        INTEGER NOT NULL,
  block_json TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
);

CREATE TABLE host_store (
  scope      TEXT NOT NULL,
  key        TEXT NOT NULL,
  value_json TEXT NOT NULL,
  PRIMARY KEY (scope, key)
);

CREATE TABLE files (
  path   TEXT PRIMARY KEY,
  parent TEXT NOT NULL,
  kind   TEXT NOT NULL CHECK (kind IN ('file', 'dir')),
  mode   INTEGER NOT NULL,
  mtime  INTEGER NOT NULL,
  size   INTEGER NOT NULL,
  sha256 TEXT
);
CREATE INDEX idx_files_parent ON files(parent);
`,
  },
]

export function migrate(db: SqlDatabase, migrations: Migration[]): void {
  db.run('CREATE TABLE IF NOT EXISTS schema_migrations (id INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL)')
  const applied = new Set(db.all<{ id: number }>('SELECT id FROM schema_migrations').map((row) => row.id))
  const pending = [...migrations].sort((a, b) => a.id - b.id).filter((migration) => !applied.has(migration.id))
  for (const migration of pending) {
    db.transaction(() => {
      for (const statement of splitStatements(migration.sql)) db.run(statement)
      db.run('INSERT INTO schema_migrations (id, name, applied_at) VALUES (?, ?, ?)', [
        migration.id,
        migration.name,
        new Date().toISOString(),
      ])
    })
  }
}

function splitStatements(sql: string): string[] {
  return sql
    .split(';')
    .map((statement) => statement.trim())
    .filter((statement) => statement.length > 0)
}
