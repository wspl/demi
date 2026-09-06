import { expect, test } from 'bun:test'
import { createRunnerWire, RUNNER_PROTOCOL_VERSION, type BackendToRunnerMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { memoryHostStore } from '@demicodes/shell/testing'
import { deferred, waitFor } from '@demicodes/utils'
import { RunnerRegistry } from '../runner/registry'
import { hashDeviceToken } from '../runner/claim-codes'
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
    const host = registry.hostFor({ deviceId: device.id, path: '/' }, 'session', memoryHostStore())
    host.startJob({ script: 'demi host shell', cwd: '/', env: { DEMI_SESSION_ID: 'session', DEMI_SHELL_ID: 'shell' } })
    const jobId = a.frames.find((frame) => frame.type === 'job_start')!.jobId
    a.handleMessage(wire.encode({ type: 'rpc_call', jobId, callId: 'call', agentSessionId: 'session', shellId: 'shell', root: 'demi', path: ['host', 'shell'], argv: [], args: {}, json: false, cwd: '/', env: {}, stdin: false }))
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


test('rpc authority comes from a live job on the authenticated device, with matching node and shell', async () => {
  let invoked = 0
  const pipes = new PipeBroker()
  const registry = new RunnerRegistry({ control: {
    ...control,
    getDeviceByTokenHash: async (hash: string) => hash === hashDeviceToken('other-token') ? { id: 'other-device' } : device,
  } as ControlService, pipes, pingIntervalMs: 0, rpc: async () => { invoked++; throw new Error('authorized handler reached') } })
  const a = socket(registry)
  const b = socket(registry)
  try {
    a.handleMessage(hello)
    b.handleMessage(wire.encode({ type: 'hello', protocol: RUNNER_PROTOCOL_VERSION, deviceToken: 'other-token', runner }))
    await waitFor(() => registry.deviceOnline(device.id) && registry.deviceOnline('other-device'))
    const host = registry.hostFor({ deviceId: device.id, path: '/' }, 'root', memoryHostStore())
    host.startJob({ script: 'demi todo list', cwd: '/', env: { DEMI_SESSION_ID: 'child', DEMI_SHELL_ID: 'shell' } })
    const jobId = a.frames.find((frame) => frame.type === 'job_start')!.jobId
    const call = { type: 'rpc_call' as const, jobId, callId: 'call', agentSessionId: 'child', shellId: 'shell', root: 'demi', path: ['demi', 'todo', 'list'], argv: ['todo', 'list'], args: {}, json: false, cwd: '/', env: {}, stdin: false }
    for (const [name, target, changes] of [
      ['wrong-device', b, {}], ['wrong-node', a, { agentSessionId: 'root' }],
      ['wrong-shell', a, { shellId: 'invented' }], ['unknown-job', a, { jobId: 'invented' }],
    ] as const) {
      target.handleMessage(wire.encode({ ...call, ...changes, callId: name }))
      await waitFor(() => target.frames.some(frame => frame.type === 'rpc_exit' && frame.callId === name))
      expect(invoked).toBe(0)
      expect(target.frames.some(frame => frame.type === 'rpc_pipes' && frame.callId === name)).toBe(false)
    }
    a.handleMessage(wire.encode(call))
    await waitFor(() => invoked === 1)
    a.handleMessage(wire.encode({ type: 'job_exit', jobId, exitCode: 0, cwd: '/' }))
    a.handleMessage(wire.encode({ ...call, callId: 'after-exit' }))
    await waitFor(() => a.frames.some(frame => frame.type === 'rpc_exit' && frame.callId === 'after-exit'))
    expect(invoked).toBe(1)
  } finally { await registry.close(); pipes.close() }
})
