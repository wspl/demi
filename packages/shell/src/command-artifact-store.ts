import type { HostStore } from './host'
import { AgentSessionCommandStorage } from './storage'

/**
 * Owns persistence of shell command artifacts: a per-scope storage cache plus the
 * set of released (tombstoned) commands. `BashEnvironment` delegates the storage
 * and release-tracking side of the `/@` virtual filesystem here, keeping only the
 * in-memory record lookups that need live command state.
 */
export class CommandArtifactStore {
  private readonly storageByScope = new Map<string, AgentSessionCommandStorage>()
  private readonly released = new Set<string>()

  constructor(private readonly store: HostStore) {}

  /** The artifact storage for a command scope, created (and cached) on first use. */
  storageFor(scopeId: string): AgentSessionCommandStorage {
    const existing = this.storageByScope.get(scopeId)
    if (existing) return existing
    const storage = new AgentSessionCommandStorage(this.store, scopeId)
    this.storageByScope.set(scopeId, storage)
    return storage
  }

  /** Whether a command's artifact has been released (tombstoned). */
  isReleased(scopeId: string, commandId: string): boolean {
    return this.released.has(this.key(scopeId, commandId))
  }

  /** Persists an artifact unless its command has already been released. */
  persist(scopeId: string, commandId: string, artifact: unknown): void {
    if (this.isReleased(scopeId, commandId)) return
    void this.storageFor(scopeId)
      .writeJson(`commands/${commandId}/artifact.json`, artifact)
      .catch(() => {})
  }

  /** Tombstones a command and removes its persisted artifact. */
  async release(scopeId: string, commandId: string): Promise<void> {
    this.released.add(this.key(scopeId, commandId))
    await this.storageFor(scopeId).delete(`commands/${commandId}/artifact.json`).catch(() => {})
  }

  private key(scopeId: string, commandId: string): string {
    return `${scopeId}\0${commandId}`
  }
}
