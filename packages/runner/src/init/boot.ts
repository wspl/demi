// PID 1 (`managed-hosts.md` § Lifecycle, "Runner is PID 1"): the init duties
// from the plan, both writable volumes under watch, then the runner itself with
// the guest user for every job, the token off the kernel command line and
// nothing of it on disk. The runner's own state lives on the ephemeral
// /run mount so the home stays untouched by the runner's own bookkeeping.
import type { Host } from '@demicodes/shell'
import { collectBytes, decodeUtf8, encodeUtf8 } from '@demicodes/utils'
import { guestBootConfig, type GuestBootConfig } from './cmdline'
import { BlockVolume } from './volume'
import {
  GUEST_LAYOUT,
  initPlan,
  resolvConf,
  runInit,
  type GuestLayout
} from './plan'

/**
 * The guest user every job runs as (`managed-hosts.md` § Lifecycle): fixed by
 * the image.
 */
export const GUEST_USER = {
  name: 'demi',
  uid: 1000,
  gid: 1000,
  homeDir: '/home/demi'
}

/**
 * Where PID 1 keeps runner.json, the socket, the command cache and job output:
 * /run, gone with the VM.
 */
export const GUEST_STATE_DIR = '/run/demi'

export interface GuestBoot {
  config: GuestBootConfig
  volumes: Record<'home' | 'system', BlockVolume>
  stateDir: string
}

/** Runs a command from the rootfs to its end with both streams collected. */
export function commandRunner(
  host: Host
): (
  command: string,
  args: string[]
) => Promise<{
  code: number | null;
  stdout: Uint8Array;
  stderr: Uint8Array
}> {
  return async (command, args) => {
    const child = await host.process.spawn!({
      command,
      args,
      env: { PATH: '/usr/sbin:/usr/bin:/sbin:/bin' }
    })
    await child.closeStdin()
    const [stdout, stderr, exit] = await Promise.all([
      collectBytes(child.stdout),
      collectBytes(child.stderr),
      child.wait()
    ])
    return { code: exit.exitCode, stdout, stderr }
  }
}

/**
 * The boot: the kernel filesystems, the upper over `/`, the home, the
 * network, the diskstats baseline — then the configuration the runner
 * starts from. Any fatal step throws, and PID 1 exiting is the VM's death
 * the backend observes.
 */
export async function bootGuest(
  host: Host,
  log: (line: string) => void,
  layout: GuestLayout = GUEST_LAYOUT
): Promise<GuestBoot> {
  const run = commandRunner(host)
  const cmdline = decodeUtf8(
    await host.fs.readFile('/proc/cmdline').catch(() => encodeUtf8(''))
  )
  // `/proc` may not be mounted yet: the plan mounts it first, and the command line is read again after.
  const config = guestBootConfig(cmdline || decodeUtf8(await readAfterProc(
    host,
    run,
    log
  )))
  await runInit(initPlan(layout, config.network), async (command, args) => {
    const result = await run(command, args)
    return { code: result.code, stderr: decodeUtf8(result.stderr) }
  }, log)
  if (config.network)
    await host.fs.writeFile(
      '/etc/resolv.conf',
      encodeUtf8(resolvConf(config.network))
    )
  const volumeIO = {
    run: (command: string, args: string[]) => command === 'resize2fs' ? run(
      'sudo',
      ['-n', command, ...args]
    ) : run(
      command,
      args
    ),
    readFile: (path: string) => host.fs.readFile(path)
  }
  const home = new BlockVolume(volumeIO, layout.homeDevice, layout.homeMount)
  const system = new BlockVolume(
    volumeIO,
    layout.systemDevice,
    `${layout.oldRoot}${layout.upperMount}`
  )
  await host.fs.mkdir(GUEST_STATE_DIR, { recursive: true })
  await host.fs.mkdir(GUEST_USER.homeDir, { recursive: true })
  const ownership = await run(
    'chown',
    [`${GUEST_USER.uid}:${GUEST_USER.gid}`, GUEST_STATE_DIR]
  )
  if (ownership.code !== 0)
    throw new Error('Cannot assign runner state to the guest user')
  // The first boot furnishes the empty home from the skeleton, as `useradd -m`
  // would (`managed-hosts.md` § First use of the home).
  if (config.firstBoot) {
    const furnished = await run('cp', ['-a', '/etc/skel/.', GUEST_USER.homeDir])
    if (furnished.code !== 0)
      throw new Error('Cannot furnish the guest home from /etc/skel')
  }
  // A freshly made image carries the backend user's ownership (`mke2fs -d`); every later boot finds the guest user's.
  await run(
    'chown',
    [
      ...(config.firstBoot ? ['-R'] : []),
      `${GUEST_USER.uid}:${GUEST_USER.gid}`,
      GUEST_USER.homeDir
    ]
  )
  return { config, volumes: { home, system }, stateDir: GUEST_STATE_DIR }
}

async function readAfterProc(
  host: Host,
  run: ReturnType<typeof commandRunner>,
  log: (line: string) => void
): Promise<Uint8Array> {
  const mounted = await run('mount', ['-t', 'proc', 'proc', '/proc'])
  if (mounted.code !== 0)
    log(
      `init: mounting /proc for the command line: ${decodeUtf8(mounted.stderr).trim()}`
    )
  return host.fs.readFile('/proc/cmdline')
}
