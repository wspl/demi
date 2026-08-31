import { Database } from 'bun:sqlite'
import { mkdirSync } from 'node:fs'
import { dirname } from 'node:path'

/**
 * The thin dual-dialect storage seam: hand-written SQL kept to the
 * SQLite/Postgres common subset behind this interface. v1 ships the
 * `bun:sqlite` driver only; the Postgres driver arrives with the scaled
 * milestone and changes nothing above this file.
 */
export interface SqlDatabase {
  run(sql: string, params?: SqlParams): void
  all<T = Record<string, unknown>>(sql: string, params?: SqlParams): T[]
  get<T = Record<string, unknown>>(sql: string, params?: SqlParams): T | null
  /** Runs `fn` atomically; nested calls join the outer transaction. */
  transaction<T>(fn: () => T): T
  close(): void
}

export type SqlParams = ReadonlyArray<string | number | bigint | boolean | null | Uint8Array>

export function openSqliteDatabase(path: string): SqlDatabase {
  if (path !== ':memory:') mkdirSync(dirname(path), { recursive: true })
  const db = new Database(path, { create: true, strict: true })
  db.run('PRAGMA journal_mode = WAL')
  db.run('PRAGMA foreign_keys = ON')
  let transactionDepth = 0
  return {
    run(sql, params = []) {
      db.run(sql, normalizeParams(params))
    },
    all<T>(sql: string, params: SqlParams = []) {
      return db.query(sql).all(...normalizeParams(params)) as T[]
    },
    get<T>(sql: string, params: SqlParams = []) {
      return (db.query(sql).get(...normalizeParams(params)) as T | null) ?? null
    },
    transaction<T>(fn: () => T): T {
      if (transactionDepth > 0) return fn()
      transactionDepth += 1
      try {
        return db.transaction(fn)()
      } finally {
        transactionDepth -= 1
      }
    },
    close() {
      db.close()
    },
  }
}

function normalizeParams(params: SqlParams): (string | number | bigint | null | Uint8Array)[] {
  return params.map((value) => (typeof value === 'boolean' ? (value ? 1 : 0) : value))
}
