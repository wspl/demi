import { expect, test } from 'bun:test'
import { bootArgs } from '../managed/firecracker/boot-args'
import { DEFAULTS, MANAGED_ENV, firecrackerConfigFromEnv } from '../managed/firecracker/config'
import { SlotPool, slotOf } from '../managed/firecracker/slots'

// The Firecracker provisioner's pure parts: slots out of the managed
// subnet, the guest's kernel command line, the configuration from the
// environment. The VM itself runs in the env-gated smoke.

test('slots: a /30 each, host .1 and guest .2, taken and given back', () => {
  const options = { subnet: '172.16.0.0/16', count: 3, tapPrefix: 'demi' }
  expect(slotOf(options, 0)).toEqual({ index: 0, tap: 'demi0', guestAddress: '172.16.0.2/30', gateway: '172.16.0.1', mac: '06:fc:00:00:00:00' })
  expect(slotOf(options, 1)).toMatchObject({ tap: 'demi1', guestAddress: '172.16.0.6/30', gateway: '172.16.0.5' })
  expect(slotOf(options, 64)).toMatchObject({ guestAddress: '172.16.1.2/30', gateway: '172.16.1.1', mac: '06:fc:00:00:00:40' })
  expect(() => slotOf({ ...options, subnet: '10.0.0.0/30' }, 1)).toThrow('does not fit')
  const pool = new SlotPool(options)
  const a = pool.take()
  const b = pool.take()
  pool.take()
  expect([a.index, b.index]).toEqual([0, 1])
  expect(() => pool.take()).toThrow('all 3 VM slots')
  pool.release(b)
  expect(pool.take().index).toBe(1)
})

test('the kernel command line the guest init reads', () => {
  const slot = slotOf({ subnet: '172.16.0.0/16', count: 8, tapPrefix: 'demi' }, 5)
  const line = bootArgs({ backendUrl: 'http://172.16.0.1:3271', deviceToken: 'tok', slot, dns: ['1.1.1.1'], firstBoot: true })
  expect(line).toBe('console=ttyS0 reboot=k panic=1 pci=off init=/demi-runner demi.backend=http://172.16.0.1:3271 demi.token=tok demi.ip=172.16.0.22/30 demi.gw=172.16.0.21 demi.dns=1.1.1.1 demi.firstboot=1')
  expect(bootArgs({ backendUrl: 'http://b', deviceToken: 't', slot, dns: [], firstBoot: false })).not.toContain('firstboot')
  expect(() => bootArgs({ backendUrl: 'http://b', deviceToken: 'a b', slot, dns: [], firstBoot: false })).toThrow('device token')
})

test('configuration from the environment: absent, direct, jailer, and the errors', () => {
  expect(firecrackerConfigFromEnv({}, '/data')).toBeNull()
  const base = { [MANAGED_ENV.firecracker]: '/opt/fc/firecracker', [MANAGED_ENV.kernel]: '/opt/fc/vmlinux', [MANAGED_ENV.rootfs]: '/opt/fc/rootfs.ext4' }
  const direct = firecrackerConfigFromEnv(base, '/data')!
  expect(direct.launch).toEqual({ mode: 'direct' })
  expect(direct).toMatchObject({ vcpus: DEFAULTS.vcpus, memMib: DEFAULTS.memMib, homeMib: DEFAULTS.homeMib, subnet: DEFAULTS.subnet, slots: DEFAULTS.slots, dns: DEFAULTS.dns, runDir: '/data/firecracker', homesDir: '/data/homes' })
  const jailer = firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.launch]: 'jailer', [MANAGED_ENV.jailer]: '/opt/fc/jailer', [MANAGED_ENV.helper]: '/usr/local/bin/demi-fc-helper', [MANAGED_ENV.uidBase]: '30000', [MANAGED_ENV.slots]: '16', [MANAGED_ENV.dns]: '9.9.9.9' }, '/data')!
  expect(jailer.launch).toEqual({ mode: 'jailer', jailer: '/opt/fc/jailer', helper: '/usr/local/bin/demi-fc-helper', chrootBase: '/srv/jailer', uidBase: 30000, gidBase: 30000 })
  expect(jailer.slots).toBe(16)
  expect(jailer.dns).toEqual(['9.9.9.9'])
  expect(() => firecrackerConfigFromEnv({ [MANAGED_ENV.firecracker]: '/fc' }, '/data')).toThrow(MANAGED_ENV.kernel)
  expect(() => firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.launch]: 'jailer' }, '/data')).toThrow(MANAGED_ENV.jailer)
  expect(() => firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.vcpus]: 'two' }, '/data')).toThrow(MANAGED_ENV.vcpus)
  expect(() => firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.launch]: 'podman' }, '/data')).toThrow('direct or jailer')
})
