import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { bootArgs } from '../managed/firecracker/boot-args'
import { DEFAULTS, MANAGED_ENV, firecrackerConfigFromEnv } from '../managed/firecracker/config'
import { FirecrackerProvisioner, type ImageTools, type ProcessControl } from '../managed/firecracker/provisioner'
import { SlotPool, slotOf } from '../managed/firecracker/slots'
import type { HomeImageStore } from '../storage/home-image-store'

// The Firecracker provisioner's pure parts: slots out of the managed
// subnet, the guest's kernel command line, the configuration from the
// environment, and the lifecycle over injected image tools and process
// control. The VM itself runs in the env-gated smoke.

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

// The lifecycle (`managed-hosts.md` § Home persistence): what a previous
// backend left is killed and saved at start, a save that fails keeps the
// working image and refuses the next boot, destroy saves before it forgets.

class MemoryStore implements HomeImageStore {
  readonly images = new Map<string, string>()
  async has(ownerKey: string): Promise<boolean> {
    return this.images.has(ownerKey)
  }
  async put(ownerKey: string, path: string): Promise<void> {
    this.images.set(ownerKey, await readFile(path, 'utf8'))
    await rm(path, { force: true })
  }
  async get(ownerKey: string, path: string): Promise<void> {
    await writeFile(path, this.images.get(ownerKey) ?? '')
  }
  async delete(ownerKey: string): Promise<void> {
    this.images.delete(ownerKey)
  }
}

const exists = (path: string) => stat(path).then(() => true, () => false)

async function fixture(options: { shrinkFails?: (path: string) => boolean } = {}) {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-fc-unit-'))
  const config = firecrackerConfigFromEnv({ [MANAGED_ENV.firecracker]: '/opt/fc/firecracker', [MANAGED_ENV.kernel]: '/opt/fc/vmlinux', [MANAGED_ENV.rootfs]: '/opt/fc/rootfs.ext4' }, dataDir)!
  const shrunk: string[] = []
  const tools: ImageTools = {
    makeHomeImage: async (homeDir, imagePath) => {
      await writeFile(imagePath, `image of ${homeDir}`)
    },
    shrinkImage: async (path) => {
      if (options.shrinkFails?.(path)) throw new Error('e2fsck exited 4: filesystem errors left uncorrected')
      shrunk.push(path)
      return (await stat(path)).size
    },
    growImage: async () => {},
  }
  const alive = new Set<number>()
  const killed: string[] = []
  const processes: ProcessControl = {
    alive: (pid) => alive.has(pid),
    kill: async (vmId, pid) => {
      killed.push(`${vmId}:${pid}`)
      alive.delete(pid)
    },
  }
  const store = new MemoryStore()
  const log: string[] = []
  const provisioner = new FirecrackerProvisioner(config, { store, tools, processes, log: (line) => log.push(line) })
  return { dataDir, config, provisioner, store, tools, alive, killed, shrunk, log }
}

test('reconcile kills the VMs a previous backend left and saves their working images; a failed save keeps the image', async () => {
  const { config, provisioner, store, alive, killed, shrunk, log } = await fixture({ shrinkFails: (path) => path.endsWith('workspace:w2.ext4') })
  // Two VMs still running from the previous process, one whose process is already gone, one stale checkpoint copy.
  await mkdir(join(config.runDir, 'vm-aaaaaaaaaaaa'), { recursive: true })
  await writeFile(join(config.runDir, 'vm-aaaaaaaaaaaa', 'vm.json'), JSON.stringify({ pid: 4242, owner: 'conversation:c1' }))
  await mkdir(join(config.runDir, 'vm-bbbbbbbbbbbb'), { recursive: true })
  await writeFile(join(config.runDir, 'vm-bbbbbbbbbbbb', 'vm.json'), JSON.stringify({ pid: 4343, owner: 'workspace:w2' }))
  await mkdir(join(config.runDir, 'vm-cccccccccccc'), { recursive: true })
  await writeFile(join(config.runDir, 'vm-cccccccccccc', 'vm.json'), JSON.stringify({ pid: 4444, owner: 'conversation:c3' }))
  alive.add(4242)
  alive.add(4343)
  const homes = join(config.runDir, 'homes')
  await mkdir(homes, { recursive: true })
  await writeFile(join(homes, 'conversation:c1.ext4'), 'c1 newest')
  await writeFile(join(homes, 'workspace:w2.ext4'), 'w2 newest')
  await writeFile(join(homes, 'conversation:c1.ext4.checkpoint-old'), 'partial copy')
  store.images.set('conversation:c1', 'c1 stale')

  await provisioner.reconcile()

  expect(killed.sort()).toEqual(['vm-aaaaaaaaaaaa:4242', 'vm-bbbbbbbbbbbb:4343'])
  expect(await exists(join(config.runDir, 'vm-aaaaaaaaaaaa'))).toBe(false)
  expect(await exists(join(config.runDir, 'vm-cccccccccccc'))).toBe(false)
  expect(await exists(join(homes, 'conversation:c1.ext4.checkpoint-old'))).toBe(false)
  // c1: saved, the working image consumed, the store now holds the newer copy.
  expect(shrunk).toEqual([join(homes, 'conversation:c1.ext4')])
  expect(store.images.get('conversation:c1')).toBe('c1 newest')
  expect(await exists(join(homes, 'conversation:c1.ext4'))).toBe(false)
  // w2: the save failed, so the image stays where it is and the store is untouched.
  expect(await exists(join(homes, 'workspace:w2.ext4'))).toBe(true)
  expect(store.images.has('workspace:w2')).toBe(false)
  expect(log.some((line) => line.includes('workspace:w2') && line.includes('not saved'))).toBe(true)

  // The next boot of w2 retries the save first and refuses when it fails again; c1's boot would proceed to the store copy.
  await expect(provisioner.wake({ kind: 'workspace', id: 'w2' }, { backendUrl: 'http://b', deviceToken: 't' })).rejects.toThrow('e2fsck exited 4')
  expect(await exists(join(homes, 'workspace:w2.ext4'))).toBe(true)
})

test('destroy saves the home before forgetting the guest; a second destroy is a no-op', async () => {
  const { config, provisioner, store } = await fixture()
  const homes = join(config.runDir, 'homes')
  await mkdir(homes, { recursive: true })
  await writeFile(join(homes, 'conversation:c9.ext4'), 'c9 work')
  await provisioner.reconcile()
  expect(store.images.get('conversation:c9')).toBe('c9 work')
  // An image left after reconcile (a save that failed) is saved by destroy too.
  await writeFile(join(homes, 'conversation:c9.ext4'), 'c9 later')
  await provisioner.destroy({ kind: 'conversation', id: 'c9' })
  expect(store.images.get('conversation:c9')).toBe('c9 later')
  expect(await exists(join(homes, 'conversation:c9.ext4'))).toBe(false)
  await provisioner.destroy({ kind: 'conversation', id: 'c9' })
  expect(provisioner.running({ kind: 'conversation', id: 'c9' })).toBe(false)
})
