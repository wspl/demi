// The Firecracker provisioner's configuration (`managed-hosts.md` §
// Provisioning), from `DEMI_MANAGED_*` in production. None set ⇒ no
// managed hosts.
import { join } from 'node:path'

export type LaunchMode =
  /** The backend spawns `firecracker` itself, unprivileged. */
  | { mode: 'direct' }
  /** The privileged helper runs the jailer; one uid per slot from `uidBase`. */
  | { mode: 'jailer'; jailer: string; helper: string; chrootBase: string; uidBase: number; gidBase: number }

export interface FirecrackerConfig {
  firecracker: string
  launch: LaunchMode
  kernel: string
  rootfs: string
  vcpus: number
  memMib: number
  /** A home image's nominal size: what an image is enlarged to at every boot. */
  homeMib: number
  subnet: string
  slots: number
  tapPrefix: string
  dns: string[]
  /** Working files per VM: the API socket, the console log, the working home image. */
  runDir: string
  /** The home-image store's directory. */
  homesDir: string
}

export const MANAGED_ENV = {
  launch: 'DEMI_MANAGED_LAUNCH',
  firecracker: 'DEMI_MANAGED_FIRECRACKER',
  jailer: 'DEMI_MANAGED_JAILER',
  helper: 'DEMI_MANAGED_HELPER',
  chrootBase: 'DEMI_MANAGED_CHROOT_BASE',
  uidBase: 'DEMI_MANAGED_UID_BASE',
  kernel: 'DEMI_MANAGED_KERNEL',
  rootfs: 'DEMI_MANAGED_ROOTFS',
  vcpus: 'DEMI_MANAGED_VCPUS',
  memMib: 'DEMI_MANAGED_MEM_MIB',
  homeMib: 'DEMI_MANAGED_HOME_MIB',
  subnet: 'DEMI_MANAGED_SUBNET',
  slots: 'DEMI_MANAGED_SLOTS',
  dns: 'DEMI_MANAGED_DNS',
} as const

export const DEFAULTS = { vcpus: 2, memMib: 2048, homeMib: 1024, subnet: '172.16.0.0/16', slots: 256, tapPrefix: 'demi', dns: ['1.1.1.1', '8.8.8.8'], chrootBase: '/srv/jailer', uidBase: 20000 }

/** The configuration the environment describes, or null when `DEMI_MANAGED_FIRECRACKER` is unset. */
export function firecrackerConfigFromEnv(env: Record<string, string | undefined>, dataDir: string): FirecrackerConfig | null {
  const firecracker = env[MANAGED_ENV.firecracker]
  if (!firecracker) return null
  const required = (name: keyof typeof MANAGED_ENV): string => {
    const value = env[MANAGED_ENV[name]]
    if (!value) throw new Error(`${MANAGED_ENV[name]} is required when ${MANAGED_ENV.firecracker} is set`)
    return value
  }
  const integer = (name: keyof typeof MANAGED_ENV, fallback: number): number => {
    const value = env[MANAGED_ENV[name]]
    if (value === undefined) return fallback
    const parsed = Number(value)
    if (!Number.isInteger(parsed) || parsed <= 0) throw new Error(`${MANAGED_ENV[name]} must be a positive integer, got ${value}`)
    return parsed
  }
  const mode = env[MANAGED_ENV.launch] ?? 'direct'
  let launch: LaunchMode
  if (mode === 'direct') launch = { mode: 'direct' }
  else if (mode === 'jailer') {
    const uidBase = integer('uidBase', DEFAULTS.uidBase)
    launch = { mode: 'jailer', jailer: required('jailer'), helper: required('helper'), chrootBase: env[MANAGED_ENV.chrootBase] ?? DEFAULTS.chrootBase, uidBase, gidBase: uidBase }
  } else throw new Error(`${MANAGED_ENV.launch} must be direct or jailer, got ${mode}`)
  return {
    firecracker,
    launch,
    kernel: required('kernel'),
    rootfs: required('rootfs'),
    vcpus: integer('vcpus', DEFAULTS.vcpus),
    memMib: integer('memMib', DEFAULTS.memMib),
    homeMib: integer('homeMib', DEFAULTS.homeMib),
    subnet: env[MANAGED_ENV.subnet] ?? DEFAULTS.subnet,
    slots: integer('slots', DEFAULTS.slots),
    tapPrefix: DEFAULTS.tapPrefix,
    dns: (env[MANAGED_ENV.dns] ?? DEFAULTS.dns.join(',')).split(',').filter((entry) => entry.length > 0),
    runDir: join(dataDir, 'firecracker'),
    homesDir: join(dataDir, 'homes'),
  }
}
