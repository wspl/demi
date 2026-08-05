import { encodeUtf8 } from '@demicodes/utils'
import type { Host } from './host'

export interface CommandArtifactFiles {
  meta: string
  stdout: string
  stderr: string
  /** Full raw bytes of a binary final stream; written once at command exit. */
  stdoutBin?: Uint8Array
}

/**
 * Writes shell command artifacts as plain files under
 * `host.commandArtifactsDir/<storageId>/<commandId>/` — one shared filesystem
 * namespace with the processes the shell spawns, so any tool (portable or
 * real) reads and searches artifacts with ordinary file operations.
 */
export class CommandArtifactStore {
  private readonly released = new Set<string>()
  // Per-command write chain: persists fire on every status transition and the
  // writes are async — chaining keeps a later snapshot from interleaving with
  // an earlier one still in flight.
  private readonly writeChains = new Map<string, Promise<void>>()

  constructor(private readonly host: Host) {}

  dirFor(commandStorageId: string, commandId: string): string {
    return `${this.host.commandArtifactsDir}/${commandStorageId}/${commandId}`
  }

  /** Whether a command's artifact has been released (removed from disk). */
  isReleased(commandStorageId: string, commandId: string): boolean {
    return this.released.has(this.key(commandStorageId, commandId))
  }

  /** Persists the artifact files unless the command has already been released. */
  persist(commandStorageId: string, commandId: string, files: CommandArtifactFiles): void {
    const key = this.key(commandStorageId, commandId)
    if (this.released.has(key)) return
    const dir = this.dirFor(commandStorageId, commandId)
    this.chain(key, async () => {
      if (this.released.has(key)) return
      const writeOptions = { cwd: this.host.defaultCwd, createParents: true }
      await this.host.fs.writeFile(`${dir}/meta.json`, encodeUtf8(files.meta), writeOptions)
      await this.host.fs.writeFile(`${dir}/stdout.txt`, encodeUtf8(files.stdout), writeOptions)
      await this.host.fs.writeFile(`${dir}/stderr.txt`, encodeUtf8(files.stderr), writeOptions)
      if (files.stdoutBin) await this.host.fs.writeFile(`${dir}/stdout.bin`, files.stdoutBin, writeOptions)
    })
  }

  /** Tombstones a command and removes its artifact directory. */
  async release(commandStorageId: string, commandId: string): Promise<void> {
    const key = this.key(commandStorageId, commandId)
    this.released.add(key)
    await new Promise<void>((resolve) => {
      this.chain(key, async () => {
        await this.host.fs
          .rm(this.dirFor(commandStorageId, commandId), { cwd: this.host.defaultCwd, recursive: true, force: true })
          .catch(() => {})
        resolve()
      })
    })
  }

  private chain(key: string, work: () => Promise<void>): void {
    const previous = this.writeChains.get(key) ?? Promise.resolve()
    const next = previous.then(work).catch(() => {})
    this.writeChains.set(key, next)
    void next.finally(() => {
      if (this.writeChains.get(key) === next) this.writeChains.delete(key)
    })
  }

  private key(commandStorageId: string, commandId: string): string {
    return `${commandStorageId}\0${commandId}`
  }
}
