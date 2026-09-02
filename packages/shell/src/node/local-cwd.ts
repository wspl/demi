import { constants } from 'node:fs'
import { open } from 'node:fs/promises'
import type { FileHandle } from 'node:fs/promises'
import { isAbsolute, posix } from 'node:path'
import type { HostCwd } from '../host'

const DIR_FLAGS = constants.O_RDONLY | constants.O_DIRECTORY

export class LocalHostCwd implements HostCwd {
  path: string
  private handle: FileHandle | undefined

  static async open(path: string): Promise<LocalHostCwd> {
    const cwd = new LocalHostCwd(path)
    cwd.handle = await open(path, DIR_FLAGS)
    return cwd
  }

  private constructor(path: string) {
    this.path = path
  }

  spawnPath(): string {
    return this.fdAnchor() ?? this.path
  }

  async chdir(path: string): Promise<void> {
    if (path === '.') return
    const target = isAbsolute(path)
      ? path
      : this.fdAnchor()
        ? `${this.fdAnchor()}/${path}`
        : posix.join(this.path, path)
    const next = await open(target, DIR_FLAGS)
    const previous = this.handle
    this.handle = next
    this.path = isAbsolute(path) ? path : posix.normalize(posix.join(this.path, path))
    await previous?.close().catch(() => {})
  }

  async snapshot(): Promise<{ restore(): void }> {
    const anchor = this.fdAnchor()
    if (!this.handle || !anchor) {
      const path = this.path
      const handle = this.handle
      return {
        restore: () => {
          this.path = path
          this.handle = handle
        },
      }
    }
    const dup = await open(anchor, DIR_FLAGS)
    const path = this.path
    return {
      restore: () => {
        const abandoned = this.handle
        this.handle = dup
        this.path = path
        if (abandoned && abandoned.fd !== dup.fd) {
          void abandoned.close().catch(() => {})
        }
      },
    }
  }

  async close(): Promise<void> {
    await this.handle?.close().catch(() => {})
    this.handle = undefined
  }

  private fdAnchor(): string | undefined {
    if (!this.handle) return undefined
    // Linux-only: macOS devfs cannot open or traverse a directory through
    // /dev/fd/N (open → ENOTDIR, /dev/fd/N/sub → ENOENT, spawn cwd → ENOTDIR),
    // so darwin falls back to logical path semantics.
    if (process.platform === 'linux') return `/proc/self/fd/${this.handle.fd}`
    return undefined
  }
}
