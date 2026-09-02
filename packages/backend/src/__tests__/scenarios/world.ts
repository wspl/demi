import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { defineProvider } from '@demicodes/provider'
import type { RunnerProtocolMessage } from '@demicodes/runner-protocol'
import { startTinyjsRunner, type TinyjsRunner } from '@demicodes/runner/testing'
import { waitFor } from '@demicodes/utils'
import { createBackend, type Backend } from '../../index'
import { ScriptedModel } from './model'
import { Driver, type Target } from './driver'

/**
 * The world: one backend over a temp data directory, its `stub` provider
 * type answered by the scripted model, and the packed tinyjs runners named
 * in `runners`, each paired as a device with a workspace at its home. One
 * world per test file, one conversation per scenario.
 */
export interface WorldOptions {
  /** Runner names; each becomes a device and a workspace. */
  runners?: string[]
  /** A fixed port, for the restart file: a runner must find the restarted process. */
  port?: number
  /** Reuse a data directory: the restart file reopens its world over the previous one. */
  dataDir?: string
}

export interface Device {
  name: string
  home: string
  stateDir: string
  runner: TinyjsRunner
  deviceId: string
  workspaceId: string
}

export interface WireFrame {
  deviceId: string
  direction: 'in' | 'out'
  message: RunnerProtocolMessage
}

/** The view budget a runner job's output frames stay within: a head and a tail. */
export const JOB_VIEW_BYTES = 2 * 32 * 1024

export class World {
  readonly drivers: Driver[] = []
  private wireCursor = 0

  private constructor(
    readonly frames: WireFrame[],
    public backend: Backend,
    readonly dataDir: string,
    readonly model: ScriptedModel,
    readonly selection: { providerId: string; model: ModelSelection },
    readonly devices: Map<string, Device>,
    private readonly port: number | undefined,
  ) {}

  static async create(options: WorldOptions = {}): Promise<World> {
    const dataDir = options.dataDir ?? (await mkdtemp(join(tmpdir(), 'demi-scenario-')))
    const model = new ScriptedModel()
    const frames: WireFrame[] = []
    const backend = await World.openBackend(dataDir, options.port, model, frames)
    const connectionId = await stubConnection(backend)
    const world = new World(frames, backend, dataDir, model, selectionFor(connectionId), new Map(), options.port)
    for (const name of options.runners ?? []) await world.pair(name)
    return world
  }

  private static openBackend(dataDir: string, port: number | undefined, model: ScriptedModel, frames: WireFrame[]): Promise<Backend> {
    return createBackend({
      dataDir,
      port: port ?? 0,
      runner: { pingIntervalMs: 0, trace: (deviceId, direction, message) => void frames.push({ deviceId, direction, message }) },
      providerTypes: {
        stub: ({ connectionId, label }) => defineProvider({ id: connectionId, displayName: label, createRuntime: () => model.runtime() }),
      },
    })
  }

  get url(): string {
    return this.backend.url
  }

  device(name: string): Device {
    const device = this.devices.get(name)
    if (!device) throw new Error(`no runner named ${name}`)
    return device
  }

  /** Starts a runner, claims its pairing code, creates its workspace. */
  async pair(name: string): Promise<Device> {
    const home = await mkdtemp(join(tmpdir(), `demi-scenario-${name}-`))
    const stateDir = await mkdtemp(join(tmpdir(), `demi-scenario-${name}-state-`))
    const runner = await startTinyjsRunner({ backendUrl: this.url, stateDir, home, name })
    await waitFor(() => runner.codes.length > 0, () => runner.log.join('\n'), { timeoutMs: 15_000 })
    const { device } = await this.api<{ device: { id: string } }>('/api/devices/claim', { code: runner.codes[0] })
    await waitFor(() => runner.statuses.includes('online'), () => runner.log.join('\n'), { timeoutMs: 10_000 })
    const { workspace } = await this.api<{ workspace: { id: string } }>('/api/workspaces', { deviceId: device.id, path: home, name: `${name} workspace` })
    const paired: Device = { name, home, stateDir, runner, deviceId: device.id, workspaceId: workspace.id }
    this.devices.set(name, paired)
    return paired
  }

  /** Stops a runner's process; `returnRunner` starts it again over the same state. */
  async killRunner(name: string): Promise<void> {
    await this.device(name).runner.stop()
  }

  async returnRunner(name: string): Promise<void> {
    const device = this.device(name)
    device.runner = await startTinyjsRunner({ backendUrl: this.url, stateDir: device.stateDir, home: device.home, name })
    await waitFor(() => device.runner.statuses.includes('online'), () => device.runner.log.join('\n'), { timeoutMs: 15_000 })
  }

  /** Closes the backend without the invariants and reopens it over the same data directory. */
  async restartBackend(): Promise<void> {
    if (this.port === undefined) throw new Error('restartBackend needs a world with a fixed port')
    for (const driver of this.drivers) await driver.detach()
    await this.backend.close()
    this.backend = await World.openBackend(this.dataDir, this.port, this.model, this.frames)
    for (const device of this.devices.values()) {
      await waitFor(() => device.runner.statuses.at(-1) === 'online', () => device.runner.log.join('\n'), { timeoutMs: 15_000 })
    }
  }

  async conversation(target: Target): Promise<Driver> {
    const driver = await Driver.open(this, target)
    this.drivers.push(driver)
    return driver
  }

  /** Frames since the previous call, per device. */
  wire(deviceName?: string): WireFrame[] {
    const frames = this.frames.slice(this.wireCursor)
    this.wireCursor = this.frames.length
    if (!deviceName) return frames
    const id = this.device(deviceName).deviceId
    return frames.filter((frame) => frame.deviceId === id)
  }

  async api<T>(path: string, body?: unknown, method = body === undefined ? 'GET' : 'POST'): Promise<T> {
    const response = await fetch(`${this.url}${path}`, {
      method,
      ...(body === undefined ? {} : { body: JSON.stringify(body), headers: { 'content-type': 'application/json' } }),
    })
    if (!response.ok) throw new Error(`${method} ${path}: HTTP ${response.status} ${await response.text()}`)
    return (await response.json()) as T
  }

  /** The teardown: the invariants over every conversation, then the processes. */
  async close(): Promise<void> {
    try {
      await this.checkInvariants()
    } finally {
      for (const driver of this.drivers) await driver.detach()
      for (const device of this.devices.values()) await device.runner.stop()
      await this.backend.close()
    }
  }

  private async checkInvariants(): Promise<void> {
    // Every script consumed.
    expect(this.model.pending(), 'scripts left unconsumed').toEqual([])
    // The cold transcript equals the live one.
    for (const driver of this.drivers) {
      const live = driver.transcript().map((block) => block.id)
      const cold = await this.api<{ blocks: Block[] }>(`/api/conversations/${driver.id}/transcript`)
      expect(cold.blocks.map((block) => block.id), `cold transcript of ${driver.label}`).toEqual(live)
    }
    // Runner sockets carried the view only; every job exited; every transfer completed.
    for (const device of this.devices.values()) {
      const of = (direction: 'in' | 'out') => this.frames.filter((f) => f.deviceId === device.deviceId && f.direction === direction).map((f) => f.message)
      const jobBytes = new Map<string, number>()
      for (const message of of('in')) {
        if (message.type === 'job_output') jobBytes.set(message.jobId, (jobBytes.get(message.jobId) ?? 0) + message.bytes.byteLength)
      }
      for (const [jobId, bytes] of jobBytes) expect(bytes, `job ${jobId} output on ${device.name}'s socket`).toBeLessThanOrEqual(JOB_VIEW_BYTES)
      const count = (direction: 'in' | 'out', ...types: string[]) => of(direction).filter((m) => types.includes(m.type)).length
      expect(count('in', 'job_exit'), `jobs exited on ${device.name}`).toBe(count('out', 'job_start'))
      expect(count('in', 'transfer_done'), `transfers completed on ${device.name}`).toBe(count('out', 'transfer_send', 'transfer_receive'))
    }
    // One ledger row per provider request.
    const usage = await this.api<{ totals: Array<{ requests: number }> }>('/api/usage')
    expect(usage.totals.reduce((sum, group) => sum + group.requests, 0), 'ledger rows').toBe(this.model.requests.length)
  }
}

function selectionFor(connectionId: string) {
  const model: ModelSelection = {
    providerId: connectionId,
    model: { id: 'scenario-model', name: 'Scenario Model', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
    thinking: null,
  }
  return { providerId: connectionId, model }
}

async function stubConnection(backend: Backend): Promise<string> {
  const response = await fetch(`${backend.url}/api/connections`, {
    method: 'POST',
    body: JSON.stringify({ type: 'stub', label: 'Stub', apiKey: 'test-key' }),
    headers: { 'content-type': 'application/json' },
  })
  if (response.status !== 201) throw new Error(`connection create failed: ${response.status}`)
  const { connection } = (await response.json()) as { connection: { id: string } }
  return connection.id
}
