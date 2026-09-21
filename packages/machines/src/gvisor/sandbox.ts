import { chmod, chown, mkdir, open, readFile, rm, rmdir, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import { managedBootSchema } from '@demicodes/runner-protocol'
import { delay, errorCode } from '@demicodes/utils'
import { atomicJson, syncFile } from '../machine-image-store'
import type { BootArgs, ManagedVolume } from '../provisioner'
import { RUNTIME_FLAGS, type GVisorConfig } from './config'
import { mountSystemOverlay, requireTool, runTool, thawFilesystem, volumeBytes } from './image-tools'
import { CloudNetwork } from './network'
import type { Slot } from './slots'

export const sandboxRecordSchema = z.strictObject({
  id: z.string().regex(/^demi-[a-zA-Z0-9_-]+$/),
  slot: z.number().int().nonnegative(),
})
export type SandboxRecord = z.infer<typeof sandboxRecordSchema>
const runtimeStateSchema = z.object({ id: z.string(), status: z.enum(['creating', 'created', 'running', 'paused', 'stopped']) })
const loopSchema = z.object({ loopdevices: z.array(z.object({ name: z.string().regex(/^\/dev\/loop\d+$/) })) })
const capabilities = ['CHOWN', 'DAC_OVERRIDE', 'FOWNER', 'FSETID', 'KILL', 'SETGID', 'SETUID', 'SETPCAP', 'NET_BIND_SERVICE', 'SYS_CHROOT', 'SETFCAP'].map(name => `CAP_${name}`)

/** Own every resource of one Cloud boot, including partial starts and recovery. */
export class Sandbox {
  readonly directory: string
  private waiter: Bun.Subprocess<'ignore', 'pipe', 'pipe'> | null = null
  exited: Promise<void> | null = null

  constructor(
    private readonly config: GVisorConfig,
    readonly record: SandboxRecord,
    private readonly work: string,
    private readonly slot: Slot,
    private readonly network: CloudNetwork,
  ) {
    this.directory = join(config.runtimeDir, record.id)
  }

  private args(args: string[]): string[] {
    return [`--root=${join(this.config.runtimeDir, 'runsc')}`, ...RUNTIME_FLAGS, ...args]
  }

  async state() {
    const result = await runTool(this.config.runsc, this.args(['list', '--format=json']))
    if (result.code !== 0) throw new Error(`Cannot inspect Cloud runtimes: ${result.stderr}`)
    const states = z.array(runtimeStateSchema).nullable().parse(JSON.parse(result.stdout))
    // runsc encodes an empty list as JSON null.
    return states?.find(state => state.id === this.record.id)
  }

  async start(base: string, boot: BootArgs): Promise<void> {
    const credentials = managedBootSchema.parse(boot)
    if (new URL(credentials.backendUrl).href !== this.config.backendUrl) throw new Error('Cloud backend differs from configured allowlist')
    await atomicJson(join(this.work, 'sandbox.json'), this.record)
    await syncFile(this.work)
    await mkdir(this.directory, { recursive: true, mode: 0o700 })
    for (const name of ['base', 'system', 'home', 'rootfs', 'credentials']) {
      await mkdir(join(this.directory, name))
    }
    await requireTool('mount', ['--bind', base, join(this.directory, 'base')])
    await requireTool('mount', ['-o', 'remount,bind,ro', join(this.directory, 'base')])
    for (const name of ['system', 'home']) {
      const image = join(this.work, `${name}.ext4`)
      const check = await runTool('e2fsck', ['-p', image])
      if (check.code !== 0 && check.code !== 1) throw new Error(`Cloud ${name} filesystem needs recovery: ${check.stderr}`)
      await requireTool('mount', ['-o', 'loop,nodev', image, join(this.directory, name)])
    }
    const system = join(this.directory, 'system')
    await mountSystemOverlay(join(this.directory, 'base'), system, join(this.directory, 'rootfs'))
    await requireTool('mount', ['-t', 'tmpfs', '-o', 'size=1m,mode=0700', 'tmpfs', join(this.directory, 'credentials')])
    const credential = join(this.directory, 'credentials', 'boot.json')
    await writeFile(credential, JSON.stringify(credentials), { mode: 0o400 })
    await chown(credential, 1000, 1000)
    const resolver = join(this.directory, 'credentials', 'resolv.conf')
    await writeFile(resolver, this.config.dns.map(address => `nameserver ${address}\n`).join(''), { mode: 0o444 })
    await chmod(resolver, 0o444)
    const hosts = join(this.directory, 'credentials', 'hosts')
    await writeFile(hosts, '127.0.0.1 localhost\n127.0.1.1 demi-cloud\n', { mode: 0o444 })
    await chmod(hosts, 0o444)
    await this.network.create(this.slot)
    const bind = (source: string, destination: string, readonly = false) => ({
      destination, type: 'bind', source, options: ['bind', ...(readonly ? ['ro'] : []), 'nodev'],
    })
    await atomicJson(join(this.directory, 'config.json'), {
      ociVersion: '1.1.0',
      root: { path: join(this.directory, 'rootfs'), readonly: false },
      process: {
        terminal: false, user: { uid: 1000, gid: 1000 }, cwd: '/home/demi',
        args: ['/usr/bin/tini', '--', '/usr/bin/demi-runner', 'run', '--managed-boot', '/run/demi-boot.json'],
        env: ['HOME=/home/demi', 'USER=demi', 'LOGNAME=demi', 'LANG=en_US.UTF-8', 'PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin'],
        capabilities: { bounding: capabilities, effective: [], permitted: [], inheritable: [], ambient: [] },
        noNewPrivileges: false,
        rlimits: [{ type: 'RLIMIT_NOFILE', hard: 65536, soft: 65536 }, { type: 'RLIMIT_NPROC', hard: 1024, soft: 1024 }],
      },
      hostname: 'demi-cloud',
      mounts: [
        { destination: '/proc', type: 'proc', source: 'proc', options: ['nosuid', 'nodev', 'noexec'] },
        { destination: '/dev', type: 'tmpfs', source: 'tmpfs', options: ['nosuid', 'mode=755', 'size=65536k'] },
        { destination: '/dev/pts', type: 'devpts', source: 'devpts', options: ['nosuid', 'noexec', 'newinstance', 'ptmxmode=0666', 'mode=0620', 'gid=5'] },
        ...['/dev/shm', '/tmp', '/run'].map(destination => ({
          destination, type: 'tmpfs', source: 'tmpfs', options: ['nosuid', 'nodev', 'size=256m', `mode=${destination === '/run' ? '0755' : '1777'}`, ...(destination === '/run' ? ['uid=1000', 'gid=1000'] : [])],
        })),
        bind(join(this.directory, 'home'), '/home'),
        bind(credential, '/run/demi-boot.json', true),
        bind(resolver, '/etc/resolv.conf', true),
        bind(hosts, '/etc/hosts', true),
      ],
      linux: {
        namespaces: [{ type: 'pid' }, { type: 'ipc' }, { type: 'uts' }, { type: 'mount' }, { type: 'network', path: `/run/netns/${this.slot.namespace}` }],
        cgroupsPath: `/demi-cloud/${this.record.id}`,
        resources: {
          cpu: { period: 100000, quota: this.config.cpus * 100000 },
          memory: { limit: this.config.memMib * 1024 ** 2, swap: this.config.memMib * 1024 ** 2 },
          pids: { limit: 1024 },
        },
        maskedPaths: ['/proc/kcore', '/proc/keys', '/proc/timer_list', '/sys'],
        readonlyPaths: ['/proc/sys', '/proc/sysrq-trigger', '/proc/irq', '/proc/bus'],
      },
    })
    // Detached sandbox processes retain stdio. A pipe would never reach EOF at startup.
    const logPath = join(this.directory, 'runtime.log')
    const log = await open(logPath, 'wx', 0o600)
    try {
      const launch = Bun.spawn([this.config.runsc, ...this.args(['run', '--detach', '--bundle', this.directory, this.record.id])], {
        stdin: 'ignore', stdout: log.fd, stderr: log.fd, timeout: 60_000, killSignal: 'SIGKILL',
      })
      if (await launch.exited !== 0) throw new Error(`Cloud start failed: ${(await readFile(logPath, 'utf8')).slice(-8192)}`)
    } finally {
      await log.close()
    }
    this.waiter = Bun.spawn([this.config.runsc, ...this.args(['wait', this.record.id])], { stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' })
    const waiter = this.waiter
    this.exited = Promise.all([waiter.exited, new Response(waiter.stdout).text(), new Response(waiter.stderr).text()]).then(([code, , error]) => {
      if (code !== 0) throw new Error(`Cloud runtime wait failed: ${error}`)
    })
  }

  async pause(): Promise<void> {
    await requireTool(this.config.runsc, this.args(['pause', this.record.id]))
  }

  async resume(): Promise<void> {
    await requireTool(this.config.runsc, this.args(['resume', this.record.id]))
  }

  async freeze(volume: ManagedVolume): Promise<void> {
    await requireTool('fsfreeze', ['--freeze', join(this.directory, volume)])
  }

  async thaw(volume: ManagedVolume): Promise<void> {
    await thawFilesystem(join(this.directory, volume))
  }

  async grow(volume: ManagedVolume, bytes: number): Promise<number> {
    const image = join(this.work, `${volume}.ext4`)
    const { truncate } = await import('node:fs/promises')
    if ((await stat(image)).size < bytes) await truncate(image, bytes)
    const loops = loopSchema.parse(JSON.parse(await requireTool('losetup', ['--json', '--output', 'NAME', '-j', image])))
    if (loops.loopdevices.length !== 1) throw new Error('Cloud volume has ambiguous loop-device ownership')
    const device = loops.loopdevices[0]!.name
    await requireTool('losetup', ['--set-capacity', device])
    await requireTool('resize2fs', [device])
    await syncFile(image)
    return volumeBytes(image)
  }

  async close(): Promise<void> {
    // Thaw before signalling: a surviving Gofer may be waiting for filesystem IO.
    for (const volume of ['system', 'home'] as const) {
      const mounted = await runTool('mountpoint', ['-q', join(this.directory, volume)])
      if (mounted.code === 0) await this.thaw(volume)
    }
    const state = await this.state()
    if (state && state.status !== 'stopped') {
      if (state.status === 'paused') await this.resume()
      const result = await runTool(this.config.runsc, this.args(['kill', '--all', this.record.id, 'TERM']))
      if (result.code !== 0 && (await this.state())?.status !== 'stopped') {
        throw new Error(`Cannot signal Cloud sandbox: ${result.stderr}`)
      }
      const deadline = Date.now() + 3000
      while (Date.now() < deadline && (await this.state())?.status === 'running') await delay(50)
    }
    if (await this.state()) await requireTool(this.config.runsc, this.args(['delete', '--force', this.record.id]))
    if (this.waiter) {
      await this.waiter.exited
      this.waiter = null
    }
    // Fence a partially created runtime even when runsc never published its state.
    const cgroup = join('/sys/fs/cgroup/demi-cloud', this.record.id)
    const exists = await stat(cgroup).then(() => true, error => {
      if (errorCode(error) === 'ENOENT') return false
      throw error
    })
    if (exists) {
      await writeFile(join(cgroup, 'cgroup.kill'), '1')
      const deadline = Date.now() + 5000
      while (!/^populated 0$/m.test(await readFile(join(cgroup, 'cgroup.events'), 'utf8'))) {
        if (Date.now() >= deadline) throw new Error('Cloud runtime writers did not terminate')
        await delay(50)
      }
      await rmdir(cgroup)
    }
    for (const name of ['rootfs', 'home', 'system', 'base', 'credentials']) {
      const mount = join(this.directory, name)
      const exists = await stat(mount).then(() => true, error => {
        if (errorCode(error) === 'ENOENT') return false
        throw error
      })
      if (!exists) continue
      const mounted = await runTool('mountpoint', ['-q', mount])
      if (mounted.code === 0) {
        if (name === 'home' || name === 'system') await this.thaw(name)
        await requireTool('umount', [mount])
      } else if (mounted.code !== 32 && !(mounted.code === 1 && mounted.stderr.includes('No such file'))) {
        throw new Error(`Cannot inspect Cloud mount ${mount}: ${mounted.stderr}`)
      }
    }
    await this.network.remove(this.slot)
    await rm(this.directory, { recursive: true, force: true })
    await rm(join(this.work, 'sandbox.json'), { force: true })
    await syncFile(this.work)
  }
}
