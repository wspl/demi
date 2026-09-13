// The Firecracker provisioner's configuration (`managed-hosts.md` §
// Provisioning), from `DEMI_MANAGED_*` in production. None set ⇒ no
// managed hosts.
import { join } from 'node:path'
import { z } from 'zod'

export type LaunchMode =
/** The backend spawns `firecracker` itself, unprivileged. */
| { mode: 'direct' }
/** The privileged helper runs the jailer; one uid per slot from `uidBase`. */
| {
  mode: 'jailer';
  jailer: string;
  helper: string;
  chrootBase: string;
  uidBase: number;
  gidBase: number
}

export interface FirecrackerConfig {
  firecracker: string
  launch: LaunchMode
  kernel: string
  rootfs: string
  vcpus: number
  memMib: number
  /** Initial writable-volume sizes; existing volumes keep their capacity. */
  systemMib: number
  homeMib: number
  subnet: string
  slots: number
  tapPrefix: string
  dns: string[]
  /**
   * Working files per VM: the API socket, the console log, both working disks.
   */
  runDir: string
  /** Immutable paired disk generations and pinned base images. */
  imagesDir: string
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
  systemMib: 'DEMI_MANAGED_SYSTEM_MIB',
  homeMib: 'DEMI_MANAGED_HOME_MIB',
  subnet: 'DEMI_MANAGED_SUBNET',
  slots: 'DEMI_MANAGED_SLOTS',
  dns: 'DEMI_MANAGED_DNS',
} as const

export const DEFAULTS = {
  vcpus: 2,
  memMib: 2048,
  // Nominal capacities, grown on demand. Publication copies each image in
  // full (no reflink on ext4), so a generation costs both capacities on the
  // host: keep these modest until the copy preserves holes.
  systemMib: 1024,
  homeMib: 1024,
  subnet: '172.16.0.0/16',
  slots: 256,
  tapPrefix: 'demi',
  dns: ['1.1.1.1', '8.8.8.8'],
  chrootBase: '/srv/jailer',
  uidBase: 20000,
}

/** A count or a size: a whole number above zero, spelled in decimal. */
const positiveInteger = z.coerce.number().int().positive()

/** Everything both launch modes read. */
const managedCommon = {
  [MANAGED_ENV.firecracker]: z.string().min(1),
  [MANAGED_ENV.kernel]: z.string().min(1),
  [MANAGED_ENV.rootfs]: z.string().min(1),
  [MANAGED_ENV.vcpus]: positiveInteger.default(DEFAULTS.vcpus),
  [MANAGED_ENV.memMib]: positiveInteger.default(DEFAULTS.memMib),
  [MANAGED_ENV.systemMib]: positiveInteger.default(DEFAULTS.systemMib),
  [MANAGED_ENV.homeMib]: positiveInteger.default(DEFAULTS.homeMib),
  [MANAGED_ENV.subnet]: z.string().min(1).default(DEFAULTS.subnet),
  [MANAGED_ENV.slots]: positiveInteger.default(DEFAULTS.slots),
  [MANAGED_ENV.dns]: z
    .string()
    .default(DEFAULTS.dns.join(','))
    .transform((value) => value.split(',').filter((entry) => entry.length > 0)),
} as const

/**
 * The `DEMI_MANAGED_*` variables, with `DEMI_MANAGED_LAUNCH` deciding which
 * ones apply: the jailer needs its binary, its helper and a uid range that
 * direct spawning has no use for.
 */
const managedEnvSchema = z.discriminatedUnion(
  MANAGED_ENV.launch,
  [
    z.object({
      [MANAGED_ENV.launch]: z.literal('direct').default('direct'),
      ...managedCommon,
    }),
    z.object({
      [MANAGED_ENV.launch]: z.literal('jailer'),
      [MANAGED_ENV.jailer]: z.string().min(1),
      [MANAGED_ENV.helper]: z.string().min(1),
      [MANAGED_ENV.chrootBase]: z.string().min(1).default(DEFAULTS.chrootBase),
      [MANAGED_ENV.uidBase]: positiveInteger.default(DEFAULTS.uidBase),
      ...managedCommon,
    }),
  ],
  `${MANAGED_ENV.launch} must be direct or jailer`,
)

type ManagedEnv = z.infer<typeof managedEnvSchema>

// TypeScript narrows a union through an element access only when the key is a
// const of literal type, so the discriminator's name gets one.
const LAUNCH = MANAGED_ENV.launch

/** The launch mode as its own variables describe it. */
function launchModeOf(env: ManagedEnv): LaunchMode {
  if (env[LAUNCH] === 'direct') {
    return { mode: 'direct' }
  }
  const uidBase = env[MANAGED_ENV.uidBase]
  return {
    mode: 'jailer',
    jailer: env[MANAGED_ENV.jailer],
    helper: env[MANAGED_ENV.helper],
    chrootBase: env[MANAGED_ENV.chrootBase],
    uidBase,
    // One uid and one gid per slot, numbered from the same base.
    gidBase: uidBase,
  }
}

/**
 * The configuration the environment describes, or null when
 * `DEMI_MANAGED_FIRECRACKER` is unset.
 */
export function firecrackerConfigFromEnv(
  env: Record<string, string | undefined>,
  dataDir: string,
): FirecrackerConfig | null {
  if (!env[MANAGED_ENV.firecracker]) {
    return null
  }
  const managed = managedEnvSchema.parse(env)
  return {
    firecracker: managed[MANAGED_ENV.firecracker],
    launch: launchModeOf(managed),
    kernel: managed[MANAGED_ENV.kernel],
    rootfs: managed[MANAGED_ENV.rootfs],
    vcpus: managed[MANAGED_ENV.vcpus],
    memMib: managed[MANAGED_ENV.memMib],
    systemMib: managed[MANAGED_ENV.systemMib],
    homeMib: managed[MANAGED_ENV.homeMib],
    subnet: managed[MANAGED_ENV.subnet],
    slots: managed[MANAGED_ENV.slots],
    tapPrefix: DEFAULTS.tapPrefix,
    dns: managed[MANAGED_ENV.dns],
    runDir: join(dataDir, 'firecracker'),
    imagesDir: join(dataDir, 'machines'),
  }
}
