import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { LocalControlService, type ControlService, type ManagedHostOwner } from '../../storage/control'
import { openSqliteDatabase } from '../../storage/database'
import { ownerKey } from '../../managed/provisioner'
import { FakeProvisioner } from './fake-provisioner'
import { itemsText } from './model'
import { World } from './world'
import { model } from './driver'

// S12 — the Cloud workspace (`managed-hosts.md` § Cloud workspace): creating
// a workspace with the Cloud device choice provisions one host owned by the
// workspace over an empty home and creates the workspace at that home;
// every conversation under it executes there; idle counts across them;
// deleting the workspace destroys the guest; a backend without a
// provisioner refuses the choice.

const PING_MS = 500
const IDLE_MS = 1_500

let world: World
let fake: FakeProvisioner
let control: ControlService
let controlDb: ReturnType<typeof openSqliteDatabase>

interface Workspace {
  id: string
  deviceId: string
  path: string
  name: string
}

beforeAll(async () => {
  fake = new FakeProvisioner()
  world = await World.create({
    pingIntervalMs: PING_MS,
    managedHosts: { provisioner: fake, config: { idleMs: IDLE_MS, hardCapMs: 60_000, checkpointIntervalMs: 60_000, sweepMs: 50, hostsPerUser: 1, bootTimeoutMs: 15_000 } },
  })
  controlDb = openSqliteDatabase(join(world.dataDir, 'control.sqlite'))
  control = new LocalControlService(controlDb)
})

afterAll(async () => {
  try {
    await world.close()
  } finally {
    await fake.close()
    controlDb.close()
  }
})

function calls(owner: ManagedHostOwner, verb: string): number {
  return fake.calls.filter((call) => call === `${verb}:${ownerKey(owner)}`).length
}

let workspace: Workspace

test('the Cloud choice provisions a host owned by the workspace and creates the workspace at its home', async () => {
  workspace = (await world.api<{ workspace: Workspace }>('/api/workspaces', { cloud: true, name: 'demo' })).workspace
  const owner: ManagedHostOwner = { kind: 'workspace', id: workspace.id }
  expect(calls(owner, 'provision')).toBe(1)
  expect(workspace.path).toBe(fake.homeOf(owner))
  expect(workspace.path).toBe(join(world.dataDir, 'staging', workspace.id))
  const device = (await control.getDevice(workspace.deviceId))!
  expect(device.kind).toBe('managed')
  expect(device.ownerWorkspaceId).toBe(workspace.id)
  const listed = await world.api<{ workspaces: Workspace[] }>('/api/workspaces')
  expect(listed.workspaces.map((entry) => entry.id)).toContain(workspace.id)
  const devices = await world.api<{ devices: Array<{ id: string }> }>('/api/devices')
  expect(devices.devices.map((entry) => entry.id)).not.toContain(workspace.deviceId)

  // One machine per user here: a second Cloud workspace is refused, and leaves no row behind.
  await expect(world.api('/api/workspaces', { cloud: true, name: 'second' })).rejects.toThrow('HTTP 409')
  expect((await world.api<{ workspaces: Workspace[] }>('/api/workspaces')).workspaces).toHaveLength(1)
  expect(await control.countManagedDevices('local')).toBe(1)
}, 30_000)

test('every conversation under the workspace executes on its host; idle counts across them; the next turn wakes it', async () => {
  const owner: ManagedHostOwner = { kind: 'workspace', id: workspace.id }
  const first = await world.conversation('hostless')
  const second = await world.conversation('hostless')
  for (const driver of [first, second]) await world.api(`/api/conversations/${driver.id}`, { workspaceId: workspace.id }, 'PATCH')

  const wrote = await first.turn({ model: [model.shell('t1', 'printf shared > note.txt && demi host current'), model.say('wrote')] })
  expect(itemsText(wrote.requests[0]!.items)).toContain('Current target: workspace "demo"')
  expect(wrote.received[0]).toContain('host: workspace "demo"')
  expect(await readFile(join(workspace.path, 'note.txt'), 'utf8')).toBe('shared')

  const read = await second.turn({ model: [model.shell('t2', 'cat note.txt && pwd'), model.say('read')] })
  expect(read.received[0]).toContain('shared')
  expect(read.received[0]).toContain(workspace.path)
  expect(calls(owner, 'provision')).toBe(1)

  // Nobody has a turn in flight and no job runs: the guest hibernates; the second conversation's next turn wakes it.
  await waitFor(() => calls(owner, 'hibernate') === 1 && !fake.running(owner), () => fake.calls.join(','), { timeoutMs: 8_000 })
  const woken = await second.turn({ model: [model.shell('t3', 'cat note.txt'), model.say('woken')] })
  expect(woken.received[0]).toContain('shared')
  expect(calls(owner, 'wake')).toBe(1)
}, 30_000)

test('deleting the workspace is refused while conversations point at it, and destroys the guest once they leave', async () => {
  const owner: ManagedHostOwner = { kind: 'workspace', id: workspace.id }
  await expect(world.api(`/api/workspaces/${workspace.id}`, undefined, 'DELETE')).rejects.toThrow('HTTP 409')
  for (const driver of world.drivers) await world.api(`/api/conversations/${driver.id}`, { workspaceId: null }, 'PATCH')
  const response = await fetch(`${world.url}/api/workspaces/${workspace.id}`, { method: 'DELETE' })
  expect(response.status).toBe(204)
  expect(calls(owner, 'destroy')).toBe(1)
  expect(fake.running(owner)).toBe(false)
  expect((await world.api<{ workspaces: Workspace[] }>('/api/workspaces')).workspaces).toHaveLength(0)
}, 30_000)

test('a backend that provisions no machines refuses the Cloud choice', async () => {
  const plain = await World.create()
  try {
    await expect(plain.api('/api/workspaces', { cloud: true, name: 'nowhere' })).rejects.toThrow('HTTP 409')
    expect((await plain.api<{ workspaces: Workspace[] }>('/api/workspaces')).workspaces).toHaveLength(0)
  } finally {
    await plain.close()
  }
}, 30_000)
