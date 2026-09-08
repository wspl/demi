import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { bootArgs } from '../managed/firecracker/boot-args'
import { DEFAULTS, MANAGED_ENV, firecrackerConfigFromEnv } from '../managed/firecracker/config'
import { FirecrackerProvisioner, type ImageTools, type ProcessControl } from '../managed/firecracker/provisioner'
import { SlotPool, slotOf } from '../managed/firecracker/slots'
import { DirMachineImageStore, atomicJson } from '../storage/machine-image-store'

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
  expect(direct).toMatchObject({ vcpus: DEFAULTS.vcpus, memMib: DEFAULTS.memMib, homeMib: DEFAULTS.homeMib, subnet: DEFAULTS.subnet, slots: DEFAULTS.slots, dns: DEFAULTS.dns, runDir: '/data/firecracker', imagesDir: '/data/machines' })
  const jailer = firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.launch]: 'jailer', [MANAGED_ENV.jailer]: '/opt/fc/jailer', [MANAGED_ENV.helper]: '/usr/local/bin/demi-fc-helper', [MANAGED_ENV.uidBase]: '30000', [MANAGED_ENV.slots]: '16', [MANAGED_ENV.dns]: '9.9.9.9' }, '/data')!
  expect(jailer.launch).toEqual({ mode: 'jailer', jailer: '/opt/fc/jailer', helper: '/usr/local/bin/demi-fc-helper', chrootBase: '/srv/jailer', uidBase: 30000, gidBase: 30000 })
  expect(jailer.slots).toBe(16)
  expect(jailer.dns).toEqual(['9.9.9.9'])
  expect(() => firecrackerConfigFromEnv({ [MANAGED_ENV.firecracker]: '/fc' }, '/data')).toThrow(MANAGED_ENV.kernel)
  expect(() => firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.launch]: 'jailer' }, '/data')).toThrow(MANAGED_ENV.jailer)
  expect(() => firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.vcpus]: 'two' }, '/data')).toThrow(MANAGED_ENV.vcpus)
  expect(() => firecrackerConfigFromEnv({ ...base, [MANAGED_ENV.launch]: 'podman' }, '/data')).toThrow('direct or jailer')
})


async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), 'demi-machine-'))
  const kernel = join(directory, 'kernel')
  const rootfs = join(directory, 'rootfs')
  await writeFile(kernel, 'kernel-v1')
  await writeFile(rootfs, 'root-v1')
  const config = firecrackerConfigFromEnv({ DEMI_MANAGED_FIRECRACKER: '/missing-firecracker', DEMI_MANAGED_KERNEL: kernel, DEMI_MANAGED_ROOTFS: rootfs }, directory)!
  const tools: ImageTools = {
    makeHomeImage: async (_home, image) => { await writeFile(image, 'empty-home') },
    makeSystemImage: async image => { await writeFile(image, 'empty-system') },
    growImage: async () => {},
  }
  const alive = new Set<number>()
  const killed: number[] = []
  const processes: ProcessControl = { alive: pid => alive.has(pid), kill: async (_id, pid) => { alive.delete(pid); killed.push(pid) } }
  const store = new DirMachineImageStore(config.imagesDir)
  const provisioner = new FirecrackerProvisioner(config, { tools, processes, store })
  return { config, provisioner, store, alive, killed }
}

test('reconciliation kills orphan VMs before publishing both working disks', async () => {
  const { config, provisioner, store, alive, killed } = await fixture()
  const vm = join(config.runDir, 'vm-orphan')
  const working = join(config.runDir, 'machines', 'device-a')
  await mkdir(vm, { recursive: true })
  await mkdir(working, { recursive: true })
  await writeFile(join(vm, 'vm.json'), JSON.stringify({ pid: 123, owner: 'device-a' }))
  alive.add(123)
  await writeFile(join(working, 'system.ext4'), 'installed-package')
  await writeFile(join(working, 'home.ext4'), 'project-files')
  await atomicJson(join(working, 'manifest.json'), { generation: 'old', baseVersion: 'base', resetId: null, systemBytes: 17, homeBytes: 13 })
  await provisioner.reconcile()
  expect(killed).toEqual([123])
  const state = (await store.read('device-a'))!
  const copied = join(config.runDir, 'readback')
  await store.copy('device-a', state, copied)
  expect(await readFile(join(copied, 'system.ext4'), 'utf8')).toBe('installed-package')
  expect(await readFile(join(copied, 'home.ext4'), 'utf8')).toBe('project-files')
  expect(await stat(working).then(() => true, () => false)).toBe(false)
})

test('reset replaces only system, pins the base, and replays an operation id without changing generation', async () => {
  const { config, provisioner, store } = await fixture()
  await provisioner.reconcile()
  const base = await provisioner.currentBaseVersion()
  await provisioner.reset('device-a', 'first', base)
  const before = (await store.read('device-a'))!
  const stage = join(config.runDir, 'edit')
  await store.copy('device-a', before, stage)
  await writeFile(join(stage, 'home.ext4'), 'precious')
  await writeFile(join(stage, 'system.ext4'), 'broken')
  await store.publish('device-a', { ...before, generation: 'edited' }, stage)
  await writeFile(config.rootfs, 'new-deployment')
  await provisioner.reset('device-a', 'reset-2', base)
  const reset = (await store.read('device-a'))!
  expect(reset.baseVersion).toBe(base)
  await store.copy('device-a', reset, stage)
  expect(await readFile(join(stage, 'home.ext4'), 'utf8')).toBe('precious')
  expect(await readFile(join(stage, 'system.ext4'), 'utf8')).toBe('empty-system')
  await provisioner.reset('device-a', 'reset-2', base)
  expect(await store.read('device-a')).toEqual(reset)
  expect(await readFile(join(config.imagesDir, 'bases', base, 'rootfs'), 'utf8')).toBe('root-v1')
})

test('a partial disk publication cannot replace the last complete generation', async () => {
  const { config, provisioner, store } = await fixture()
  await provisioner.reconcile()
  await provisioner.reset('device-a', 'initial', await provisioner.currentBaseVersion())
  const before = (await store.read('device-a'))!
  const stage = join(config.runDir, 'partial')
  await mkdir(stage)
  await writeFile(join(stage, 'system.ext4'), 'new-system')
  await expect(store.publish('device-a', { ...before, generation: 'partial' }, stage)).rejects.toThrow()
  expect(await store.read('device-a')).toEqual(before)
  expect(await readFile(join(stage, 'system.ext4'), 'utf8')).toBe('new-system')
})

test('published checkpoints retain only the current and previous complete generations', async () => {
  const { config, provisioner, store } = await fixture()
  await provisioner.reconcile()
  await provisioner.reset('device-a', 'first', await provisioner.currentBaseVersion())
  const state = (await store.read('device-a'))!
  const stage = join(config.runDir, 'checkpoint-source')
  await store.copy('device-a', state, stage)
  await store.publish('device-a', { ...state, generation: 'second' }, stage)
  await store.publish('device-a', { ...state, generation: 'third' }, stage)
  expect(await stat(join(config.imagesDir, 'device-a', 'generations', state.generation)).then(() => true, () => false)).toBe(false)
  const previous = join(config.runDir, 'previous-readback')
  await store.copy('device-a', { ...state, generation: 'second' }, previous)
  expect(await readFile(join(previous, 'home.ext4'), 'utf8')).toBe('empty-home')
  expect((await store.read('device-a'))?.generation).toBe('third')
})


test('failed JSON serialization preserves the published file and removes its temporary file', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'demi-atomic-json-'))
  const path = join(directory, 'current.json')
  try {
    await atomicJson(path, { generation: 'complete' })
    await expect(atomicJson(path, { unserializable: 1n })).rejects.toThrow()
    expect(JSON.parse(await readFile(path, 'utf8'))).toEqual({ generation: 'complete' })
    expect(await readdir(directory)).toEqual(['current.json'])
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
