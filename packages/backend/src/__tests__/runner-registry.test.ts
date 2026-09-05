import { expect, test } from 'bun:test'
import { createRunnerWire, RUNNER_PROTOCOL_VERSION, type BackendToRunnerMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { deferred, waitFor } from '@demicodes/utils'
import { RunnerRegistry } from '../runner/registry'
import { PipeBroker } from '../runner/pipes'
import type { ControlService } from '../storage/control'

const wire = createRunnerWire(msgpackCodec)
const runner = { name: 'runner', platform: 'test', version: '0', identity: { uid: 1, gid: 1, hostname: 'runner', homeDir: '/home/runner' } }
const hello = wire.encode({ type: 'hello', protocol: RUNNER_PROTOCOL_VERSION, deviceToken: 'token', runner })
const device = { id: 'device' }
const control = { getDeviceByTokenHash: async () => device, touchDeviceSeen: async () => {} } as unknown as ControlService

function socket(registry: RunnerRegistry) {
  const frames: BackendToRunnerMessage[] = []
  const handle = registry.openSocket({ send: (frame) => frames.push(wire.decodeBackendToRunner(frame)), close() {} })
  return { ...handle, frames }
}

test('concurrent token hellos bind only one socket, including duplicate hello on the same socket', async () => {
  const manifest = deferred<unknown>()
  const registry = new RunnerRegistry({ control, manifest: () => manifest.promise, pingIntervalMs: 0, log() {} })
  const a = socket(registry)
  const b = socket(registry)
  a.handleMessage(hello)
  a.handleMessage(hello)
  b.handleMessage(hello)
  manifest.resolve({})
  await waitFor(() => a.frames.length + b.frames.length === 3)
  expect([...a.frames, ...b.frames].filter((frame) => frame.type === 'hello_ok')).toHaveLength(1)
  expect([...a.frames, ...b.frames].filter((frame) => frame.type === 'hello_error')).toEqual([
    expect.objectContaining({ code: 'already_connected' }),
  ])
  await registry.close()
})

test('a socket closed during hello or registry shutdown never becomes online', async () => {
  for (const shutdown of [false, true]) {
    const manifest = deferred<unknown>()
    const started = deferred<void>()
    const registry = new RunnerRegistry({ control, manifest: () => { started.resolve(); return manifest.promise }, pingIntervalMs: 0 })
    const a = socket(registry)
    a.handleMessage(hello)
    await started.promise
    if (shutdown) await registry.close()
    else a.handleClose()
    manifest.resolve({})
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(registry.deviceOnline(device.id)).toBe(false)
    expect(a.frames).toEqual([])
    await registry.close()
  }
})

test('a claim whose socket closes rolls back the undeliverable device token', async () => {
  const created = deferred<never>()
  const deleted: string[] = []
  const registry = new RunnerRegistry({ control: {
    ...control,
    createDevice: () => created.promise,
    deleteDevice: async (id: string) => { deleted.push(id) },
  } as ControlService, pingIntervalMs: 0 })
  const a = socket(registry)
  a.handleMessage(wire.encode({ type: 'hello', protocol: RUNNER_PROTOCOL_VERSION, runner }))
  const pending = a.frames.find((frame) => frame.type === 'claim_pending')!
  const claim = registry.claim('user', pending.claimToken)
  a.handleClose()
  created.resolve(device as never)
  expect(await claim).toEqual({ ok: false, code: 'invalid_code' })
  expect(deleted).toEqual([device.id])
  expect(registry.deviceOnline(device.id)).toBe(false)
  await registry.close()
})

test('rpc cancellation and runner disconnect abort the handler and release its live stdin', async () => {
  for (const disconnect of [false, true]) {
    const pipes = new PipeBroker()
    const started = deferred<void>()
    const stopped = deferred<void>()
    let ended = false
    const registry = new RunnerRegistry({ control, pipes, pingIntervalMs: 0, rpc: async (_call, io) => {
      const input = (async () => { for await (const _chunk of io.stdinStream) {} ended = true })()
      started.resolve()
      await new Promise<void>((resolve) => io.signal.addEventListener('abort', () => resolve(), { once: true }))
      await input
      stopped.resolve()
      return 130
    } })
    const a = socket(registry)
    a.handleMessage(hello)
    await waitFor(() => registry.deviceOnline(device.id))
    a.handleMessage(wire.encode({ type: 'rpc_call', callId: 'call', agentSessionId: 'session', shellId: 'shell', root: 'demi', path: ['host', 'shell'], argv: [], args: {}, json: false, cwd: '/', env: {}, stdin: false }))
    await started.promise
    if (disconnect) a.handleClose()
    else a.handleMessage(wire.encode({ type: 'rpc_cancel', callId: 'call' }))
    await stopped.promise
    expect(ended).toBe(true)
    if (!disconnect) await waitFor(() => a.frames.some((frame) => frame.type === 'rpc_exit' && frame.exitCode === 130))
    await registry.close()
    pipes.close()
  }
})
