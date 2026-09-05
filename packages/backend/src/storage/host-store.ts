import type { HostStore } from '@demicodes/shell'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { SqlDatabase } from './database'

/**
 * The DB-backed `HostStore` composed into every Host the backend hands the
 * harness. A single-statement upsert gives `writeJson` the atomicity callers
 * rely on (every observable row state is a complete document); `list` is one
 * indexed range scan.
 */
export class DbHostStore implements HostStore {
  constructor(
    private readonly db: SqlDatabase,
    private readonly scope: string,
  ) {}

  async readJson<T>(key: string): Promise<T | null> {
    const row = this.db.get<{ value_json: string }>(
      'SELECT value_json FROM host_store WHERE scope = ? AND key = ?',
      [this.scope, key],
    )
    return row ? parsePortableJson<T>(row.value_json) : null
  }

  async writeJson<T>(key: string, value: T): Promise<void> {
    this.db.run(
      'INSERT INTO host_store (scope, key, value_json) VALUES (?, ?, ?) ON CONFLICT (scope, key) DO UPDATE SET value_json = excluded.value_json',
      [this.scope, key, stringifyPortableJson(value)],
    )
  }

  async delete(key: string): Promise<void> {
    this.db.run('DELETE FROM host_store WHERE scope = ? AND key = ?', [this.scope, key])
  }

  async list(prefix: string): Promise<string[]> {
    const rows = this.db.all<{ key: string }>(
      "SELECT key FROM host_store WHERE scope = ? AND key LIKE ? ESCAPE '\\' ORDER BY key",
      [this.scope, `${escapeLike(prefix)}%`],
    )
    return rows.map((row) => row.key)
  }
}

function escapeLike(value: string): string {
  return value.replace(/[\\%_]/g, (match) => `\\${match}`)
}
