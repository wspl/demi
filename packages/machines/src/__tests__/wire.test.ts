import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { RemoteProvisioner } from '../client'
import type {
  BootArgs,
  MachineImageState,
  ManagedHostProvisioner,
  ManagedVolume
} from '../provisioner'
import { serveMachines, type MachineServer } from '../server'

/** Records every call and answers from a script; deaths are raised by the test. */
class ScriptedProvisioner implements ManagedHostProvisioner {
  readonly calls: unknown[][] = []
  readonly deaths: Array<(deviceId: string) => void> = []
  failNext: string | null = null
  state: MachineImageState | null = null
  /** When set, the next base-version call never answers. */
  hang: Promise<never> | null = null

  private record(...call: unknown[]): void {
    this.calls.push(call)
    if (this.failNext) {
      const message = this.failNext
      this.failNext = null
      throw new Error(message)
    }
  }

  async reconcile(): Promise<void> {
    this.record('reconcile')
  }

  async currentBaseVersion(): Promise<string> {
    this.record('currentBaseVersion')
    if (this.hang) {
      return this.hang
    }
    return 'base-1'
  }

  async imageState(deviceId: string): Promise<MachineImageState | null> {
    this.record('imageState', deviceId)
    return this.state
  }

  async wake(deviceId: string, boot: BootArgs): Promise<void> {
    this.record('wake', deviceId, boot)
  }

  async hibernate(deviceId: string): Promise<void> {
    this.record('hibernate', deviceId)
  }

  async checkpoint(deviceId: string): Promise<void> {
    this.record('checkpoint', deviceId)
  }

  async growVolume(
    deviceId: string,
    volume: ManagedVolume,
    bytes: number
  ): Promise<void> {
    this.record('growVolume', deviceId, volume, bytes)
  }

  async reset(
    deviceId: string,
    operationId: string,
    baseVersion: string
  ): Promise<void> {
    this.record('reset', deviceId, operationId, baseVersion)
  }

  async close(): Promise<void> {
    this.record('close')
  }

  onDeath(listener: (deviceId: string) => void): void {
    this.deaths.push(listener)
  }
}

let directory: string
let socketPath: string
let provisioner: ScriptedProvisioner
let server: MachineServer
const logs: string[] = []

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), 'demi-machines-'))
  socketPath = join(directory, 'machines.sock')
  provisioner = new ScriptedProvisioner()
  server = await serveMachines({ socketPath, provisioner, log: line => logs.push(line) })
})

afterEach(async () => {
  await server.close()
  await rm(directory, { recursive: true, force: true })
  logs.length = 0
})

test('every provisioner call crosses the socket with its arguments and result', async () => {
  const remote = new RemoteProvisioner({ socketPath, log: line => logs.push(line) })
  provisioner.state = {
    generation: 'gen-1',
    baseVersion: 'base-1',
    resetId: null,
    systemBytes: 1024,
    homeBytes: 2048,
  }
  expect(await remote.currentBaseVersion()).toBe('base-1')
  expect(await remote.imageState('dev-1')).toEqual(provisioner.state)
  await remote.wake('dev-1', { backendUrl: 'http://backend', deviceToken: 'tok' })
  await remote.checkpoint('dev-1')
  await remote.growVolume('dev-1', 'home', 4096)
  await remote.reset('dev-1', 'op-1', 'base-2')
  await remote.hibernate('dev-1')
  expect(provisioner.calls).toEqual([
    ['currentBaseVersion'],
    ['imageState', 'dev-1'],
    ['wake', 'dev-1', { backendUrl: 'http://backend', deviceToken: 'tok' }],
    ['checkpoint', 'dev-1'],
    ['growVolume', 'dev-1', 'home', 4096],
    ['reset', 'dev-1', 'op-1', 'base-2'],
    ['hibernate', 'dev-1'],
  ])
  await remote.close()
})

test('a failing call rejects with the manager’s message; the connection stays usable', async () => {
  const remote = new RemoteProvisioner({ socketPath, log: line => logs.push(line) })
  provisioner.failNext = 'no such machine'
  await expect(remote.hibernate('dev-9')).rejects.toThrow('no such machine')
  expect(await remote.currentBaseVersion()).toBe('base-1')
  await remote.close()
})

test('concurrent calls are answered by id, whatever order they finish in', async () => {
  const remote = new RemoteProvisioner({ socketPath, log: line => logs.push(line) })
  const results = await Promise.all([
    remote.currentBaseVersion(),
    remote.imageState('dev-1'),
    remote.currentBaseVersion(),
  ])
  expect(results).toEqual(['base-1', null, 'base-1'])
  await remote.close()
})

test('a death reaches every listener on every connection', async () => {
  const first = new RemoteProvisioner({ socketPath, log: line => logs.push(line) })
  const second = new RemoteProvisioner({ socketPath, log: line => logs.push(line) })
  const seen: string[] = []
  first.onDeath(id => seen.push(`first:${id}`))
  second.onDeath(id => seen.push(`second:${id}`))
  await first.reconcile()
  await second.reconcile()
  for (const listener of provisioner.deaths) {
    listener('dev-1')
  }
  await Bun.sleep(50)
  expect(seen.sort()).toEqual(['first:dev-1', 'second:dev-1'])
  await first.close()
  await second.close()
})

test('close reconciles the manager and disconnects; the next call reconnects', async () => {
  const remote = new RemoteProvisioner({ socketPath, log: line => logs.push(line) })
  await remote.currentBaseVersion()
  await remote.close()
  expect(provisioner.calls).toEqual([['currentBaseVersion'], ['reconcile']])
  expect(await remote.currentBaseVersion()).toBe('base-1')
  await remote.close()
})

test('a manager that goes away fails the calls in flight and is dialed again later', async () => {
  const remote = new RemoteProvisioner({ socketPath, log: line => logs.push(line) })
  provisioner.hang = new Promise<never>(() => {})
  const inFlight = remote.currentBaseVersion()
    .then(() => 'answered', (error: Error) => error.message)
  await Bun.sleep(20)
  await server.close()
  expect(await inFlight).toContain('Machine manager unavailable')
  provisioner.hang = null
  server = await serveMachines({ socketPath, provisioner, log: line => logs.push(line) })
  expect(await remote.currentBaseVersion()).toBe('base-1')
  await remote.close()
})

test('a malformed frame drops that connection without touching the provisioner', async () => {
  const socket = await Bun.connect({
    unix: socketPath,
    socket: { data() {}, close() {} },
  })
  socket.write('{"id":"1","op":"wake","params":{}}\n')
  await Bun.sleep(50)
  expect(provisioner.calls).toEqual([])
  expect(logs.some(line => line.includes('bad frame'))).toBe(true)
  socket.end()
})
