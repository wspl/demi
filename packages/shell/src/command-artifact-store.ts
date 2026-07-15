import type { HostStore } from './host'
import { AgentSessionCommandStorage } from './storage'

/**
 * Owns persistence of shell command artifacts: a per-storage-id cache plus the
 * set of released (tombstoned) commands. `BashEnvironment` delegates the storage
 * and release-tracking side of the `/@` virtual filesystem here, keeping only the
 * in-memory record lookups that need live command state.
 */
export class CommandArtifactStore {
  private readonly storageById = new Map<string, AgentSessionCommandStorage>()
  private readonly released = new Set<string>()

  constructor(private readonly store: HostStore) {}

  /** The artifact storage for one agent session or anonymous shell. */
  storageFor(commandStorageId: string): AgentSessionCommandStorage {
    const existing = this.storageById.get(commandStorageId)
    if (existing) return existing
    const storage = new AgentSessionCommandStorage(this.store, commandStorageId)
    this.storageById.set(commandStorageId, storage)
    return storage
  }

  /** Whether a command's artifact has been released (tombstoned). */
  isReleased(commandStorageId: string, commandId: string): boolean {
    return this.released.has(this.key(commandStorageId, commandId))
  }

  /** Persists an artifact unless its command has already been released. */
  persist(commandStorageId: string, commandId: string, artifact: unknown): void {
    if (this.isReleased(commandStorageId, commandId)) return
    void this.storageFor(commandStorageId)
      .writeJson(`commands/${commandId}/artifact.json`, artifact)
      .catch(() => {})
  }

  /** Tombstones a command and removes its persisted artifact. */
  async release(commandStorageId: string, commandId: string): Promise<void> {
    this.released.add(this.key(commandStorageId, commandId))
    await this.storageFor(commandStorageId).delete(`commands/${commandId}/artifact.json`).catch(() => {})
  }

  private key(commandStorageId: string, commandId: string): string {
    return `${commandStorageId}\0${commandId}`
  }
}
