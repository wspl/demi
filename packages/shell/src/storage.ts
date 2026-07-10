import type { CommandStorage } from './command'
import type { HostStore } from './host'

export class AgentSessionCommandStorage implements CommandStorage {
  private readonly agentSessionPrefix: string

  constructor(
    private readonly store: HostStore,
    agentSessionId: string,
  ) {
    validateAgentSessionId(agentSessionId)
    // Everything a session persists (checkpoint, command artifacts, blobs)
    // lives under the one agent-sessions/<id>/ prefix.
    this.agentSessionPrefix = `agent-sessions/${agentSessionId}/`
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
    return keys.map((key) => (key.startsWith(this.agentSessionPrefix) ? key.slice(this.agentSessionPrefix.length) : key))
  }

  private key(key: string): string {
    validateStorageKey(key)
    return `${this.agentSessionPrefix}${key}`
  }
}

function validateAgentSessionId(agentSessionId: string): void {
  if (
    !agentSessionId ||
    agentSessionId.includes('\0') ||
    /[\\/]/.test(agentSessionId) ||
    agentSessionId === '.' ||
    agentSessionId === '..'
  ) {
    throw new Error(`Invalid command storage agent session id: ${agentSessionId}`)
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
