// What PID 1 does before it is a runner (`managed-hosts.md` § Lifecycle,
// "Runner is PID 1"): the kernel filesystems, the ephemeral upper over `/`,
// the home, the network. Every step is a command from the rootfs — mount,
// pivot_root, ip — so this module is a plan of spawns, run by `runInit`
// and checked in tests without a kernel.
import type { GuestNetwork } from './cmdline'

export interface InitStep {
  command: string
  args: string[]
  /** The step's failure is fatal for the boot (default); a tolerated step is logged and skipped. */
  tolerated?: boolean
}

export interface GuestLayout {
  /** The block device carrying the owner's home image. */
  homeDevice: string
  /** Where it is mounted; the guest user's home lives under it. */
  homeMount: string
  /** The tmpfs that becomes the overlay's upper and work directories. */
  upperMount: string
  /** Where the overlay is assembled before it becomes `/`. */
  newRoot: string
  /** Where the read-only rootfs remains reachable after the pivot, under the new root. */
  oldRoot: string
}

export const GUEST_LAYOUT: GuestLayout = {
  homeDevice: '/dev/vdb',
  homeMount: '/home',
  upperMount: '/run/upper',
  newRoot: '/run/newroot',
  oldRoot: '/oldroot',
}

/** The kernel's own filesystems, first: everything after reads `/proc`. `/dev` is the kernel's (`CONFIG_DEVTMPFS_MOUNT`). */
export function kernelMounts(): InitStep[] {
  return [
    { command: 'mount', args: ['-t', 'proc', 'proc', '/proc'] },
    { command: 'mount', args: ['-t', 'sysfs', 'sys', '/sys'] },
    { command: 'mount', args: ['-t', 'tmpfs', 'run', '/run'] },
  ]
}

/**
 * The ephemeral upper (`managed-hosts.md` § Images): a tmpfs as the upper
 * and work directories of an overlay over the read-only rootfs, assembled
 * under `newRoot`, then made `/` by `pivot_root`. `/proc`, `/sys` and
 * `/dev` move into the new root before the pivot; `/run` cannot — the new
 * root lives under it — so it stays with the old root (the overlay keeps
 * it alive) and the new root gets a fresh `/run`. After the pivot the old
 * root sits at `oldRoot`, still read-only, still the source of every binary.
 */
export function upperOverlay(layout: GuestLayout): InitStep[] {
  const { upperMount, newRoot, oldRoot } = layout
  return [
    { command: 'mkdir', args: ['-p', upperMount, newRoot] },
    { command: 'mount', args: ['-t', 'tmpfs', 'upper', upperMount] },
    { command: 'mkdir', args: ['-p', `${upperMount}/upper`, `${upperMount}/work`] },
    { command: 'mount', args: ['-t', 'overlay', 'overlay', '-o', `lowerdir=/,upperdir=${upperMount}/upper,workdir=${upperMount}/work`, newRoot] },
    { command: 'mkdir', args: ['-p', `${newRoot}${oldRoot}`] },
    ...['/proc', '/sys', '/dev'].map((dir) => ({ command: 'mount', args: ['--move', dir, `${newRoot}${dir}`] })),
    { command: 'pivot_root', args: [newRoot, `${newRoot}${oldRoot}`] },
    { command: 'mount', args: ['-t', 'tmpfs', 'run', '/run'] },
  ]
}

/**
 * The home image, read-write, journal replayed by the mount itself after a
 * hibernate's `kill -9`; then grown into its backing file, which the
 * backend re-enlarged to the nominal size after the shrink at hibernate.
 */
export function homeMount(layout: GuestLayout): InitStep[] {
  return [
    { command: 'mkdir', args: ['-p', layout.homeMount] },
    { command: 'mount', args: ['-t', 'ext4', layout.homeDevice, layout.homeMount] },
    { command: 'resize2fs', args: [layout.homeDevice], tolerated: true },
    { command: 'mount', args: ['-t', 'tmpfs', 'tmp', '/tmp'] },
  ]
}

/** The guest's name (`/etc/hosts` in the rootfs resolves it, which sudo insists on). */
export const GUEST_HOSTNAME = 'demi'

/** One interface, one address, one default route, from the command line; the nameservers are a file the caller writes. */
export function networkSteps(network: GuestNetwork, iface = 'eth0'): InitStep[] {
  return [
    { command: 'hostname', args: [GUEST_HOSTNAME], tolerated: true },
    { command: 'ip', args: ['link', 'set', 'lo', 'up'] },
    { command: 'ip', args: ['link', 'set', iface, 'up'] },
    { command: 'ip', args: ['addr', 'add', network.address, 'dev', iface] },
    { command: 'ip', args: ['route', 'add', 'default', 'via', network.gateway, 'dev', iface] },
  ]
}

export function resolvConf(network: GuestNetwork): string {
  return network.dns.map((server) => `nameserver ${server}\n`).join('')
}

/** The whole boot, in order: kernel filesystems, the upper, the home, the network. */
export function initPlan(layout: GuestLayout, network: GuestNetwork | null): InitStep[] {
  return [...kernelMounts(), ...upperOverlay(layout), ...homeMount(layout), ...(network ? networkSteps(network) : [])]
}

/** Runs a spawn and reports its exit and output; what `runInit` needs of a process facet. */
export type StepRunner = (command: string, args: string[]) => Promise<{ code: number | null; stderr: string }>

/** Runs the plan step by step; a failed step that is not tolerated ends the boot with its stderr. */
export async function runInit(plan: InitStep[], run: StepRunner, log: (line: string) => void): Promise<void> {
  for (const step of plan) {
    const result = await run(step.command, step.args)
    if (result.code === 0) continue
    const line = `init: ${step.command} ${step.args.join(' ')} exited ${result.code ?? 'by signal'}: ${result.stderr.trim()}`
    if (step.tolerated) {
      log(line)
      continue
    }
    throw new Error(line)
  }
}
