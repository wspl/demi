import type { CommandStorage } from './command'

export type DemiStore = CommandStorage

export class SessionCommandStorage implements CommandStorage {
  private readonly sessionPrefix: string

  constructor(
    private readonly store: DemiStore,
    sessionId: string,
  ) {
    validateSessionId(sessionId)
    this.sessionPrefix = `${sessionId}/`
  }

  readJson<T>(key: string): Promise<T | null> {
    return this.store.readJson(this.key(key))
  }

  writeJson<T>(key: string, value: T): Promise<void> {
    return this.store.writeJson(this.key(key), value)
  }

  delete(key: string): Promise<void> {
    return this.store.delete(this.key(key))
  }

  async list(prefix: string): Promise<string[]> {
    const scopedPrefix = this.key(prefix)
    const keys = await this.store.list(scopedPrefix)
    return keys.map((key) => (key.startsWith(this.sessionPrefix) ? key.slice(this.sessionPrefix.length) : key))
  }

  private key(key: string): string {
    validateStorageKey(key)
    return `${this.sessionPrefix}${key}`
  }
}

function validateSessionId(sessionId: string): void {
  if (!sessionId || sessionId.includes('\0') || /[\\/]/.test(sessionId) || sessionId === '.' || sessionId === '..') {
    throw new Error(`Invalid command storage session id: ${sessionId}`)
  }
}

function validateStorageKey(key: string): void {
  if (key.includes('\0')) throw new Error(`Invalid CommandStorage key: ${key}`)
  if (key.startsWith('/') || /^[A-Za-z]:[\\/]/.test(key)) {
    throw new Error(`CommandStorage keys must be relative: ${key}`)
  }
  for (const segment of key.split(/[\\/]+/)) {
    if (segment === '..') throw new Error(`CommandStorage keys must not contain path traversal: ${key}`)
  }
}
