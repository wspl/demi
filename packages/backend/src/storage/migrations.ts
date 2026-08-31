import type { SqlDatabase } from './database'

/**
 * Numbered migrations, applied in order inside one transaction each. SQL stays
 * in the SQLite/Postgres common subset: TEXT ids, TEXT ISO timestamps, INTEGER
 * booleans, no dialect-specific column types or defaults.
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

export const MIGRATIONS: Migration[] = [
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
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users(id),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
  token_hash   TEXT NOT NULL,
  claimed_at   TEXT NOT NULL,
  last_seen_at TEXT
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
  connection_id TEXT,
  model_id      TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE INDEX idx_conversations_user ON conversations(user_id, archived, updated_at);

CREATE TABLE connections (
  id            TEXT PRIMARY KEY,
  owner_user_id TEXT REFERENCES users(id),
  type          TEXT NOT NULL,
  label         TEXT NOT NULL,
  config        TEXT NOT NULL,
  created_at    TEXT NOT NULL
);

CREATE TABLE usage_ledger (
  id              TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users(id),
  conversation_id TEXT NOT NULL,
  connection_id   TEXT NOT NULL,
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

CREATE TABLE host_store (
  scope      TEXT NOT NULL,
  key        TEXT NOT NULL,
  value_json TEXT NOT NULL,
  PRIMARY KEY (scope, key)
);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
`,
  },
]

export function migrate(db: SqlDatabase, migrations: Migration[] = MIGRATIONS): void {
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
