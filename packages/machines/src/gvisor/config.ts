import { isAbsolute, join } from 'node:path'
import ipaddr from 'ipaddr.js'
import { z } from 'zod'
import runtimeRelease from '../../runtime/release.json'

const positive = z.coerce.number().int().positive()
const absolute = z.string().refine(isAbsolute, 'must be an absolute path')
export const subnetSchema = z.string().refine(value => {
  if (!ipaddr.IPv4.isValidCIDRFourPartDecimal(value)) return false
  const [address, prefix] = ipaddr.IPv4.parseCIDR(value)
  return prefix >= 8 && prefix <= 30 &&
    address.toString() === ipaddr.IPv4.networkAddressFromCIDR(value).toString()
}, 'must be an aligned IPv4 network with prefix /8 through /30')

const environmentSchema = z.strictObject({
  DEMI_MANAGED_RUNSC: absolute,
  DEMI_MANAGED_IMAGE: absolute,
  DEMI_MANAGED_BACKEND_URL: z.url({ protocol: /^https?$/ }),
  DEMI_MANAGED_CPUS: positive.default(2),
  DEMI_MANAGED_MEM_MIB: positive.default(2048),
  DEMI_MANAGED_SYSTEM_MIB: positive.default(1024),
  DEMI_MANAGED_HOME_MIB: positive.default(1024),
  DEMI_MANAGED_SUBNET: subnetSchema.default('172.30.0.0/16'),
  DEMI_MANAGED_SLOTS: positive.max(16384).default(256),
  DEMI_MANAGED_DNS: z.string().transform(value => value.split(',')).pipe(
    z.array(z.ipv4().refine(value => !['unspecified', 'loopback', 'multicast', 'broadcast'].includes(
      ipaddr.IPv4.parse(value).range()
    ), 'resolver must be reachable IPv4')).min(1)
  ),
})

/** Read the sole Cloud runtime's configuration; old runtime settings are errors. */
export function gvisorConfigFromEnv(env: Record<string, string | undefined>, dataDir: string) {
  const config = environmentSchema.parse(Object.fromEntries(
    Object.entries(env).filter(([name, value]) => name.startsWith('DEMI_MANAGED_') && value !== undefined)
  ))
  const [, prefix] = ipaddr.IPv4.parseCIDR(config.DEMI_MANAGED_SUBNET)
  if (config.DEMI_MANAGED_SLOTS * 4 > 2 ** (32 - prefix)) {
    throw new Error('DEMI_MANAGED_SLOTS exceeds DEMI_MANAGED_SUBNET capacity')
  }
  return {
    runsc: config.DEMI_MANAGED_RUNSC,
    image: config.DEMI_MANAGED_IMAGE,
    backendUrl: new URL(config.DEMI_MANAGED_BACKEND_URL).href,
    cpus: config.DEMI_MANAGED_CPUS,
    memMib: config.DEMI_MANAGED_MEM_MIB,
    systemMib: config.DEMI_MANAGED_SYSTEM_MIB,
    homeMib: config.DEMI_MANAGED_HOME_MIB,
    subnet: config.DEMI_MANAGED_SUBNET,
    slots: config.DEMI_MANAGED_SLOTS,
    dns: config.DEMI_MANAGED_DNS,
    dataDir: absolute.parse(dataDir),
    runDir: join(dataDir, 'working'),
    imagesDir: join(dataDir, 'images'),
    runtimeDir: '/run/demi-machines',
  }
}

export type GVisorConfig = ReturnType<typeof gvisorConfigFromEnv>
export const RUNTIME_RELEASE = process.arch === 'arm64'
  ? runtimeRelease.arm64Version
  : `release-${runtimeRelease.upstream}`
export const RUNTIME_FLAGS = [
  '--platform=systrap', '--network=sandbox', '--overlay2=none',
  '--file-access=shared', '--file-access-mounts=shared', '--allow-suid=true',
  '--directfs=true',
]
