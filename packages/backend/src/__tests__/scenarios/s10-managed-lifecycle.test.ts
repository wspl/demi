import { mkdtemp, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { startTinyjsRunner } from '@demicodes/runner/testing'
import { delay, waitFor } from '@demicodes/utils'
import { LocalControlService, type ControlService, type ManagedHostOwner } from '../../storage/control'
import { openSqliteDatabase } from '../../storage/database'
import { RUNNER_PROTOCOL_VERSION, createRunnerWire, type BackendToRunnerMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { ownerKey } from '../../managed/provisioner'
import { FakeProvisioner } from './fake-provisioner'
import { itemsText } from './model'
import { World } from './world'
import { model } from './driver'

// S10 — the managed-host lifecycle over the fake provisioner: provision and
// bind, the idle rule hibernating and the next turn waking with a fresh
// token, the periodic checkpoint, jobs pinning the host until the hard cap,
// the crash-loop guard, the per-user cap, owner-scoped use, a token-less
// managed hello refused, archive destroying the guest.

// The ping carries `pong.jobs`; it must be slow enough that a manifest install on a debug runner never misses two.
const PING_MS = 500
const IDLE_MS = 1_500
const HARD_CAP_MS = 4_000
const CHECKPOINT_MS = 700

let world: World
let fake: FakeProvisioner
let control: ControlService
let controlDb: ReturnType<typeof openSqliteDatabase>

beforeAll(async () => {
  fake = new FakeProvisioner()
  world = await World.create({
    pingIntervalMs: PING_MS,
    managedHosts: {
      provisioner: fake,
      config: { idleMs: IDLE_MS, hardCapMs: HARD_CAP_MS, checkpointIntervalMs: CHECKPOINT_MS, sweepMs: 50, crashLoop: { deaths: 2, windowMs: 60_000 }, hostsPerUser: 1, bootTimeoutMs: 15_000 },
    },
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

/** Binds a conversation to a managed device the way the upgrade will (checkpoint 3): the pointer and the pending switch. */
async function bind(conversationId: string, deviceId: string): Promise<void> {
  const won = await control.switchConversationTarget(
    conversationId,
    { workspaceId: null, hostDeviceId: null },
    { workspaceId: null, hostDeviceId: deviceId },
    { from: { kind: 'hostless' }, to: { kind: 'host', deviceId } },
    { departed: null, arrivingDeviceId: deviceId },
  )
  expect(won).toBe(true)
}

function calls(owner: ManagedHostOwner, verb: string): number {
  return fake.calls.filter((call) => call === `${verb}:${ownerKey(owner)}`).length
}

async function tokenOf(owner: ManagedHostOwner): Promise<string> {
  return (await readFile(join(fake.guests.get(ownerKey(owner))!.stateDir, 'runner-token'), 'utf8')).trim()
}

test('provision and bind, idle → hibernate, the next turn wakes with a fresh token, the checkpoint clock', async () => {
  const driver = await world.conversation('hostless')
  const owner: ManagedHostOwner = { kind: 'conversation', id: driver.id }
  const home = await mkdtemp(join(tmpdir(), 'demi-s10-home-'))
  const device = await world.backend.managedHosts!.provision(owner, world.backend.session.user.id, home)
  expect(device.kind).toBe('managed')
  expect(device.name).toBe('cloud')
  expect(device.ownerConversationId).toBe(driver.id)
  const listed = await world.api<{ devices: Array<{ id: string }> }>('/api/devices')
  expect(listed.devices.map((entry) => entry.id)).not.toContain(device.id)
  const firstToken = await tokenOf(owner)

  await bind(driver.id, device.id)
  const first = await driver.turn({ model: [model.shell('t1', 'printf hi > note.txt && demi host current && demi host list'), model.say('one')] })
  expect(itemsText(first.requests[0]!.items)).toContain(`Current target: the machine "cloud" (host ${device.id})`)
  expect(first.received[0]).toContain('host: machine "cloud"')
  expect(first.received[0]).toContain(`cloud  ${device.id}  online  ${home}  (main)`)
  expect(await readFile(join(home, 'note.txt'), 'utf8')).toBe('hi')

  // No turn, no jobs: the idle window passes and the guest is hibernated; the checkpoint clock fired before it.
  await waitFor(() => calls(owner, 'hibernate') === 1 && !fake.running(owner), () => fake.calls.join(','), { timeoutMs: 8_000 })
  expect(calls(owner, 'checkpoint')).toBeGreaterThanOrEqual(1)
  // The `sync` went before the kill; a directory home never reports itself untouched.
  const frames = world.wire().filter((frame) => frame.deviceId === device.id && ['sync', 'sync_done'].includes(frame.message.type))
  expect(frames.map((frame) => `${frame.direction}:${frame.message.type}`)).toEqual(['out:sync', 'in:sync_done'])
  expect(fake.guests.get(ownerKey(owner))!.reports).toEqual([false])

  // Simultaneous needs join one wake, including token rotation.
  await Promise.all([world.backend.managedHosts!.ensureRunning(device), world.backend.managedHosts!.ensureRunning(device)])
  expect(calls(owner, 'wake')).toBe(1)
  // The next action uses the same home with the new token.
  const second = await driver.turn({ model: [model.shell('t2', 'cat note.txt'), model.say('two')] })
  expect(second.received[0]).toContain('hi')
  expect(calls(owner, 'wake')).toBe(1)
  expect(await tokenOf(owner)).not.toBe(firstToken)
  await waitFor(() => calls(owner, 'hibernate') === 2, () => fake.calls.join(','), { timeoutMs: 8_000 })
}, 30_000)

test('running jobs pin the host past the idle window; the hard cap reclaims it anyway', async () => {
  const driver = await world.conversation('hostless')
  const owner: ManagedHostOwner = { kind: 'conversation', id: driver.id }
  // The one machine per user is taken; this conversation cannot get its own.
  const other = await mkdtemp(join(tmpdir(), 'demi-s10-other-'))
  await expect(world.backend.managedHosts!.provision(owner, world.backend.session.user.id, other)).rejects.toMatchObject({ code: 'host_limit' })

  // On the first conversation's machine: a job that outlives its tool call keeps the host up.
  const first = world.drivers[0]!
  const firstOwner: ManagedHostOwner = { kind: 'conversation', id: first.id }
  const hibernates = calls(firstOwner, 'hibernate')
  const started = await first.turn({ model: [model.shell('t3', 'sleep 3; echo done', 200), model.say('running')] })
  expect(started.received[0]).toContain('status: running')
  await delay(IDLE_MS * 2)
  expect(calls(firstOwner, 'hibernate')).toBe(hibernates)
  await waitFor(() => calls(firstOwner, 'hibernate') === hibernates + 1, () => fake.calls.join(','), { timeoutMs: 8_000 })

  // A job with no turn behind it cannot pin the host past the hard cap.
  const capped = await first.turn({ model: [model.shell('t4', 'sleep 30; echo never', 200), model.say('long')] })
  expect(capped.received[0]).toContain('status: running')
  const before = Date.now()
  await waitFor(() => calls(firstOwner, 'hibernate') === hibernates + 2 && !fake.running(firstOwner), () => fake.calls.join(','), { timeoutMs: HARD_CAP_MS + 3_000 })
  expect(Date.now() - before).toBeLessThan(HARD_CAP_MS + 1_500)
}, 30_000)

test('owner-scoped: another conversation cannot use the machine; a managed runner without a token is refused', async () => {
  const first = world.drivers[0]!
  const device = (await control.getManagedDevice({ kind: 'conversation', id: first.id }))!
  const intruder = await world.conversation('hostless')
  await bind(intruder.id, device.id)
  const refused = await intruder.turn({ model: [model.shell('t5', 'cat note.txt'), model.say('denied')] })
  expect(refused.received[0]).toContain('bound to another owner')

  const stateDir = await mkdtemp(join(tmpdir(), 'demi-s10-noname-state-'))
  const home = await mkdtemp(join(tmpdir(), 'demi-s10-noname-home-'))
  const runner = await startTinyjsRunner({ backendUrl: world.url, stateDir, home, name: 'stray', managed: true })
  await waitFor(() => runner.statuses.includes('rejected'), () => runner.log.join('\n'), { timeoutMs: 10_000 })
  expect(runner.codes).toEqual([])
  await runner.stop()
}, 30_000)

test('crash loop: two deaths in the window stop the automatic restart, and the model is told', async () => {
  const first = world.drivers[0]!
  const owner: ManagedHostOwner = { kind: 'conversation', id: first.id }
  const woken = await first.turn({ model: [model.shell('t6', 'echo up'), model.say('up')] })
  expect(woken.received[0]).toContain('up')
  await fake.kill(owner)
  const again = await first.turn({ model: [model.shell('t7', 'echo up again'), model.say('up again')] })
  expect(again.received[0]).toContain('up again')
  await fake.kill(owner)
  const stuck = await first.turn({ model: [model.shell('t8', 'echo never'), model.say('stuck')] })
  expect(stuck.received[0]).toContain('died 2 times')
  expect(fake.running(owner)).toBe(false)
}, 30_000)

test('the home growth handshake: home_grow reaches the provisioner and home_grown answers with the size', async () => {
  // The guest is off after the crash loop; its current token admits a socket standing in for the runner.
  const first = world.drivers[0]!
  const owner: ManagedHostOwner = { kind: 'conversation', id: first.id }
  const wire = createRunnerWire(msgpackCodec)
  const received: BackendToRunnerMessage[] = []
  const socket = new WebSocket(`${world.url.replace(/^http/, 'ws')}/api/runner`)
  socket.binaryType = 'arraybuffer'
  socket.onmessage = (event) => received.push(wire.decodeBackendToRunner(new Uint8Array(event.data as ArrayBuffer)))
  await new Promise<void>((resolve, reject) => {
    socket.onopen = () => resolve()
    socket.onerror = () => reject(new Error('runner socket failed'))
  })
  socket.send(
    wire.encode({
      type: 'hello',
      protocol: RUNNER_PROTOCOL_VERSION,
      deviceToken: await tokenOf(owner),
      runner: { name: 'stand-in', platform: 'test', version: '0', identity: { uid: 1000, gid: 1000, hostname: 'guest', homeDir: '/home/demi' }, managed: true },
    }),
  )
  await waitFor(() => received.some((message) => message.type === 'hello_ok'), () => JSON.stringify(received), { timeoutMs: 5_000 })
  const bytes = 2 * 1024 ** 3
  socket.send(wire.encode({ type: 'home_grow', bytes }))
  await waitFor(() => received.some((message) => message.type === 'home_grown'), () => JSON.stringify(received), { timeoutMs: 5_000 })
  expect(received.find((message) => message.type === 'home_grown')).toEqual({ type: 'home_grown', bytes })
  expect(fake.calls).toContain(`grow:${ownerKey(owner)}:${bytes}`)
  socket.close()
}, 30_000)

test('archiving the owner destroys the guest', async () => {
  const first = world.drivers[0]!
  const owner: ManagedHostOwner = { kind: 'conversation', id: first.id }
  await world.api(`/api/conversations/${first.id}`, { archived: true }, 'PATCH')
  expect(calls(owner, 'destroy')).toBe(1)
  expect(fake.running(owner)).toBe(false)
  expect(await control.getManagedDevice(owner)).not.toBeNull()
}, 30_000)
