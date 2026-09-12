// The Firecracker provisioner's configuration (`managed-hosts.md` §
// Provisioning), from `DEMI_MANAGED_*` in production. None set ⇒ no
// managed hosts. Other managed settings require the binary to be set.
import { join } from 'node:path'

import { z } from 'zod'

const pathSchema = z.string().regex(/\S/, 'must not be blank')
const positiveIntegerSchema = z.int().positive()
const launchModeSchema = z.discriminatedUnion('mode', [
  z.strictObject({ mode: z.literal('direct') }),
  z.strictObject({
    mode: z.literal('jailer'),
    jailer: pathSchema,
    helper: pathSchema,
    chrootBase: pathSchema,
    uidBase: positiveIntegerSchema.max(4294967294),
    gidBase: positiveIntegerSchema.max(4294967294),
  }),
])
const subnetSchema = z.cidrv4().refine((value) => {
  const [address, prefix] = value.split('/')
  const bits = Number(prefix)
  const base = address!
    .split('.')
    .reduce((sum, part) => sum * 256 + Number(part), 0)
  return bits <= 30 && base % 2 ** (32 - bits) === 0
}, 'must be an aligned IPv4 network containing /30 slots')

export const firecrackerConfigSchema = z
  .strictObject({
    firecracker: pathSchema,
    launch: launchModeSchema,
    kernel: pathSchema,
    rootfs: pathSchema,
    vcpus: positiveIntegerSchema.max(255),
    memMib: positiveIntegerSchema,
    systemMib: positiveIntegerSchema,
    homeMib: positiveIntegerSchema,
    subnet: subnetSchema,
    slots: positiveIntegerSchema,
    tapPrefix: pathSchema,
    dns: z.array(z.ipv4()).min(1),
    runDir: pathSchema,
    imagesDir: pathSchema,
  })
  .superRefine((value, context) => {
    const bits = Number(value.subnet.split('/')[1])
    if (value.slots > 2 ** (30 - bits)) {
      context.addIssue({
        code: 'custom',
        path: ['slots'],
        message: 'exceeds subnet capacity',
      })
    }
    if (
      value.launch.mode === 'jailer' &&
      value.launch.uidBase + value.slots - 1 > 4294967294
    ) {
      context.addIssue({
        code: 'custom',
        path: ['launch', 'uidBase'],
        message: 'slot identities exceed uid range',
      })
    }
  })
export type LaunchMode = z.infer<typeof launchModeSchema>
export type FirecrackerConfig = z.infer<typeof firecrackerConfigSchema>

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

/**
 * The configuration the environment describes, or null when
 * `DEMI_MANAGED_FIRECRACKER` is unset.
 */
export function firecrackerConfigFromEnv(
  env: Record<string, string | undefined>,
  dataDir: string,
): FirecrackerConfig | null {
  const read = <T>(name: keyof typeof MANAGED_ENV, schema: z.ZodType<T>): T => {
    const result = schema.safeParse(env[MANAGED_ENV[name]])
    if (!result.success) {
      throw new Error(
        `${MANAGED_ENV[name]}: ${result.error.issues.map((issue) => issue.message).join('; ')}`,
      )
    }
    return result.data
  }
  if (env[MANAGED_ENV.firecracker] === undefined) {
    if (Object.values(MANAGED_ENV).some((name) => env[name] !== undefined)) {
      throw new Error(
        `${MANAGED_ENV.firecracker} is required with managed configuration`,
      )
    }
    return null
  }
  const integer = (name: keyof typeof MANAGED_ENV, fallback: number): number =>
    read(
      name,
      z
        .string()
        .regex(/^[0-9]+$/)
        .transform(Number)
        .pipe(positiveIntegerSchema)
        .optional(),
    ) ?? fallback
  const mode =
    read(
      'launch',
      z
        .enum(['direct', 'jailer'], {
          error: 'must be direct or jailer',
        })
        .optional(),
    ) ?? 'direct'
  let launch: LaunchMode
  if (mode === 'jailer') {
    const uidBase = integer('uidBase', DEFAULTS.uidBase)
    launch = {
      mode,
      jailer: read('jailer', pathSchema),
      helper: read('helper', pathSchema),
      chrootBase:
        read('chrootBase', pathSchema.optional()) ?? DEFAULTS.chrootBase,
      uidBase,
      gidBase: uidBase,
    }
  } else {
    for (const name of ['jailer', 'helper', 'chrootBase', 'uidBase'] as const) {
      if (env[MANAGED_ENV[name]] !== undefined) {
        throw new Error(`${MANAGED_ENV[name]} requires jailer launch mode`)
      }
    }
    launch = { mode }
  }
  return firecrackerConfigSchema.parse({
    firecracker: read('firecracker', pathSchema),
    launch,
    kernel: read('kernel', pathSchema),
    rootfs: read('rootfs', pathSchema),
    vcpus: integer('vcpus', DEFAULTS.vcpus),
    memMib: integer('memMib', DEFAULTS.memMib),
    systemMib: integer('systemMib', DEFAULTS.systemMib),
    homeMib: integer('homeMib', DEFAULTS.homeMib),
    subnet: read('subnet', subnetSchema.optional()) ?? DEFAULTS.subnet,
    slots: integer('slots', DEFAULTS.slots),
    tapPrefix: DEFAULTS.tapPrefix,
    dns: read(
      'dns',
      z
        .string()
        .transform((value) => value.split(','))
        .pipe(z.array(z.ipv4()).min(1))
        .optional(),
    ) ?? [...DEFAULTS.dns],
    runDir: join(dataDir, 'firecracker'),
    imagesDir: join(dataDir, 'machines'),
  })
}
