import { open, readFile, rm, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import { errorCode } from '@demicodes/utils'
import { atomicJson } from '../machine-image-store'
import type { GVisorConfig } from './config'
import { requireTool, runTool } from './image-tools'

/** Keep the manager's storage mounts reachable until crash recovery has fenced writers. */
export class MountNamespace {
  private readonly path: string
  private readonly owner: string

  constructor(private readonly config: GVisorConfig) {
    this.path = join(config.runtimeDir, 'mount-namespace')
    this.owner = join(config.runtimeDir, 'mount-namespace-owner.json')
  }

  private host(args: string[]): Promise<string> {
    return requireTool('nsenter', ['--mount=/proc/1/ns/mnt', '--', ...args])
  }

  async recover(): Promise<void> {
    const exists = await stat(this.path).then(() => true, error => {
      if (errorCode(error) === 'ENOENT') return false
      throw error
    })
    if (!exists) return
    const mounted = await runTool('nsenter', ['--mount=/proc/1/ns/mnt', '--', 'mountpoint', '-q', this.path])
    if (mounted.code === 0) {
      const owner = z.strictObject({ dataDir: z.string() }).parse(JSON.parse(await readFile(this.owner, 'utf8')))
      if (owner.dataDir !== this.config.dataDir) {
        throw new Error(`Recover the previous Cloud manager with its original state directory: ${owner.dataDir}`)
      }
      // This new process has inherited the host's pinned namespace handle.
      await requireTool('nsenter', [
        '--mount=/proc/1/ns/mnt', '--', 'nsenter', `--mount=${this.path}`, '--', process.execPath, '--conditions', 'development',
        z.string().min(1).parse(process.argv[1]), '--recover-namespace',
      ], undefined, 180_000)
      await this.release()
    } else if (mounted.code !== 32 && !(mounted.code === 1 && mounted.stderr.includes('No such file'))) {
      throw new Error(`Cannot inspect saved Cloud mount namespace: ${mounted.stderr}`)
    }
  }

  async pin(): Promise<void> {
    // nsfs rejects a persistent mount-namespace handle on a shared mount.
    const parent = join(this.path, '..')
    const mounted = await runTool('nsenter', ['--mount=/proc/1/ns/mnt', '--', 'mountpoint', '-q', parent])
    if (mounted.code === 32) await this.host(['mount', '--bind', parent, parent])
    else if (mounted.code !== 0) throw new Error('Cannot prepare Cloud namespace handle mount')
    await this.host(['mount', '--make-private', parent])
    const handle = await open(this.path, 'w', 0o600)
    await handle.close()
    await atomicJson(this.owner, { dataDir: this.config.dataDir })
    await this.host(['mount', '--bind', `/proc/${process.pid}/ns/mnt`, this.path])
  }

  async release(): Promise<void> {
    await this.host(['umount', this.path])
    // A replacement may also have inherited a private copy of the handle mount.
    if ((await runTool('mountpoint', ['-q', this.path])).code === 0) {
      await requireTool('umount', [this.path])
    }
    await rm(this.path, { force: true })
    await rm(this.owner, { force: true })
  }
}
