// One VM process in either launch mode (`managed-hosts.md` § Provisioning):
// `direct` spawns Firecracker as this process's child; `jailer` spawns the
// privileged helper through sudo, which prepares the chroot and runs the
// jailer, and stays as the VM's parent so its exit is the VM's death in
// both modes alike.
import { mkdir, rm } from 'node:fs/promises'
import { basename, join } from 'node:path'
import { FirecrackerApi } from './api'
import type { FirecrackerConfig } from './config'
import type { Slot } from './slots'

export interface VmStart {
  vmId: string
  slot: Slot
  /** The working home image on the host. */
  homeImage: string
  bootArgs: string
}

export interface RunningVm {
  api: FirecrackerApi
  /** The path Firecracker knows the home image by (chroot-relative under the jailer). */
  homePathInVm: string
  /** Resolves with the exit code (or null on a signal) once the VM process is gone. */
  exited: Promise<number | null>
  kill(): Promise<void>
}

const API_READY_MS = 10_000

/** Starts the process, waits for its API, configures and boots the guest. */
export async function startVm(config: FirecrackerConfig, start: VmStart, log: (line: string) => void): Promise<RunningVm> {
  const vmDir = join(config.runDir, start.vmId)
  await mkdir(vmDir, { recursive: true })
  const console = Bun.file(join(vmDir, 'console.log'))
  let socketPath: string
  let paths: { kernel: string; rootfs: string; home: string }
  let child: ReturnType<typeof Bun.spawn>
  let kill: () => Promise<void>
  if (config.launch.mode === 'direct') {
    socketPath = join(vmDir, 'api.sock')
    await rm(socketPath, { force: true })
    paths = { kernel: config.kernel, rootfs: config.rootfs, home: start.homeImage }
    child = Bun.spawn([config.firecracker, '--api-sock', socketPath, '--id', start.vmId], { stdout: console, stderr: console, stdin: 'ignore' })
    kill = async () => {
      child.kill('SIGKILL')
    }
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
    kill = async () => {
      const killer = Bun.spawn(['sudo', '-n', launch.helper, 'vm', 'kill', '--id', start.vmId, '--chroot-base', launch.chrootBase], { stdout: 'ignore', stderr: 'ignore' })
      await killer.exited
    }
  }
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
  return { api, homePathInVm: paths.home, exited, kill }
}
