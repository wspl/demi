// One VM process in either launch mode (`managed-hosts.md` § Provisioning):
// `direct` spawns Firecracker as this process's child; `jailer` spawns the
// privileged helper through sudo, which prepares the chroot and runs the
// jailer, and stays as the VM's parent so its exit is the VM's death in
// both modes alike.
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { basename, join } from 'node:path'
import process from 'node:process'
import { errorCode } from '@demicodes/utils'
import { FirecrackerApi } from './api'
import type { FirecrackerConfig } from './config'
import type { Slot } from './slots'

export interface VmStart {
  /** Short and unique: it names the run directory and the jail; the API socket path under it must fit a unix socket address. */
  vmId: string
  slot: Slot
  /** The working home image on the host. */
  homeImage: string
  bootArgs: string
  /** Who the VM belongs to, for the record a later backend process reads. */
  owner: string
}

/**
 * Written beside a VM's socket and console as soon as its process exists:
 * what a later backend process needs to kill a VM this one left running.
 */
export interface VmRecord {
  pid: number
  owner: string
}

const VM_RECORD = 'vm.json'

export interface RunningVm {
  api: FirecrackerApi
  id: string
  /** The process this backend spawned: Firecracker itself, or the helper that parents it; alive as long as the VM is. */
  pid: number
  /** The path Firecracker knows the home image by (chroot-relative under the jailer). */
  homePathInVm: string
  /** Resolves with the exit code (or null on a signal) once the VM process is gone. */
  exited: Promise<number | null>
  kill(): Promise<void>
}

const API_READY_MS = 10_000

/** The directory of one VM's working files under the run directory. */
export function vmDirectory(config: FirecrackerConfig, vmId: string): string {
  return join(config.runDir, vmId)
}

export async function readVmRecord(config: FirecrackerConfig, vmId: string): Promise<VmRecord | null> {
  try {
    const parsed = JSON.parse(await readFile(join(vmDirectory(config, vmId), VM_RECORD), 'utf8')) as Partial<VmRecord>
    return typeof parsed.pid === 'number' && typeof parsed.owner === 'string' ? { pid: parsed.pid, owner: parsed.owner } : null
  } catch {
    return null
  }
}

/** The record removed once the process is known to be gone, so no later start tries to kill it. */
export function removeVmRecord(config: FirecrackerConfig, vmId: string): Promise<void> {
  return rm(join(vmDirectory(config, vmId), VM_RECORD), { force: true })
}

/**
 * Kills the VM `vmId` whose process is `pid`: Firecracker by signal in
 * `direct` mode; through the helper in `jailer` mode, where the pid this
 * backend holds is the helper's and the jail records Firecracker's own.
 */
export async function killVm(config: FirecrackerConfig, vmId: string, pid: number): Promise<void> {
  if (config.launch.mode === 'direct') {
    try {
      process.kill(pid, 'SIGKILL')
    } catch (error) {
      if (errorCode(error) !== 'ESRCH') throw error
    }
    return
  }
  const killer = Bun.spawn(['sudo', '-n', config.launch.helper, 'vm', 'kill', '--id', vmId, '--chroot-base', config.launch.chrootBase], { stdout: 'ignore', stderr: 'ignore' })
  await killer.exited
}

/** Whether a process exists: `kill(pid, 0)` answers, permission denied included (the helper runs as root). */
export function processAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return errorCode(error) !== 'ESRCH'
  }
}

/** Starts the process, waits for its API, configures and boots the guest. */
export async function startVm(config: FirecrackerConfig, start: VmStart, log: (line: string) => void): Promise<RunningVm> {
  const vmDir = vmDirectory(config, start.vmId)
  await mkdir(vmDir, { recursive: true })
  const console = Bun.file(join(vmDir, 'console.log'))
  let socketPath: string
  let paths: { kernel: string; rootfs: string; home: string }
  let child: ReturnType<typeof Bun.spawn>
  if (config.launch.mode === 'direct') {
    socketPath = join(vmDir, 'api.sock')
    await rm(socketPath, { force: true })
    paths = { kernel: config.kernel, rootfs: config.rootfs, home: start.homeImage }
    child = Bun.spawn([config.firecracker, '--api-sock', socketPath, '--id', start.vmId], { stdout: console, stderr: console, stdin: 'ignore' })
  } else {
    const launch = config.launch
    const chroot = join(launch.chrootBase, basename(config.firecracker), start.vmId, 'root')
    socketPath = join(chroot, 'run', 'firecracker.socket')
    paths = { kernel: '/vmlinux', rootfs: '/rootfs.ext4', home: '/home.ext4' }
    const uid = launch.uidBase + start.slot.index
    const gid = launch.gidBase + start.slot.index
    const helperArgs = [
      'vm', 'start',
      '--id', start.vmId,
      '--jailer', launch.jailer,
      '--firecracker', config.firecracker,
      '--chroot-base', launch.chrootBase,
      '--uid', String(uid),
      '--gid', String(gid),
      '--backend-gid', String(process.getgid?.() ?? 0),
      '--kernel', config.kernel,
      '--rootfs', config.rootfs,
      '--home', start.homeImage,
    ]
    child = Bun.spawn(['sudo', '-n', launch.helper, ...helperArgs], { stdout: console, stderr: console, stdin: 'ignore' })
  }
  const pid = child.pid
  const record: VmRecord = { pid, owner: start.owner }
  await writeFile(join(vmDir, VM_RECORD), JSON.stringify(record))
  const kill = () => killVm(config, start.vmId, pid)
  const exited: Promise<number | null> = child.exited.then((code) => (child.signalCode ? null : code))
  const api = new FirecrackerApi(socketPath)
  try {
    await Promise.race([
      api.ready(API_READY_MS),
      exited.then((code) => {
        throw new Error(`the VM process exited with ${code ?? 'a signal'} before its API came up (see ${join(vmDir, 'console.log')})`)
      }),
    ])
    await api.configure({
      vcpus: config.vcpus,
      memMib: config.memMib,
      kernelPath: paths.kernel,
      bootArgs: start.bootArgs,
      rootfsPath: paths.rootfs,
      homePath: paths.home,
      tap: start.slot.tap,
      mac: start.slot.mac,
    })
    await api.start()
  } catch (error) {
    await kill().catch(() => {})
    throw error
  }
  log(`vm ${start.vmId} started on ${start.slot.tap} (${start.slot.guestAddress})`)
  return { api, id: start.vmId, pid, homePathInVm: paths.home, exited, kill }
}
