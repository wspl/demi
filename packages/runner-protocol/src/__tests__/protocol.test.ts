import { expect, test } from 'bun:test'
import {
  RUNNER_PROTOCOL_VERSION,
  createRunnerWire,
  gitChangeSchema,
  type RunnerProtocolMessage
} from '../index'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'

const wire = createRunnerWire(msgpackCodec)

test('runner messages round-trip through the MessagePack wire', () => {
  const runnerToBackend = new Set([
    'hello',
    'pong',
    'fs_ok',
    'fs_error',
    'spawn_output',
    'spawn_exit',
    'rpc_call',
    'rpc_cancel',
    'pipe_done',
    'net_opened',
    'net_error'
  ])
  const roundTrip = (message: RunnerProtocolMessage): RunnerProtocolMessage =>
    runnerToBackend.has(message.type)
      ? wire.decodeRunnerToBackend(wire.encode(message))
      : wire.decodeBackendToRunner(wire.encode(message))

  const hello: RunnerProtocolMessage = {
    type: 'hello',
    protocol: RUNNER_PROTOCOL_VERSION,
    deviceToken: 'token',
    runner: {
      name: 'dev-box',
      platform: 'darwin',
      version: '1.0.0',
      identity: { uid: 501, gid: 20, hostname: 'mac', homeDir: '/Users/dev' }
    },
  }
  expect(roundTrip(hello)).toEqual(hello)

  // Uint8Array file bytes and Date args are native wire types.
  const call: RunnerProtocolMessage = {
    type: 'fs_utimes',
    id: 'c1',
    path: '/tmp/x',
    atime: new Date('2026-08-31T00:00:00Z'),
    mtime: new Date('2026-08-31T01:00:00.250Z'),
    cwd: '/tmp',
  }
  const decodedCall = roundTrip(call) as Extract<RunnerProtocolMessage, { type: 'fs_utimes' }>
  expect(decodedCall.atime).toBeInstanceOf(Date)
  expect(decodedCall.atime.toISOString()).toBe('2026-08-31T00:00:00.000Z')
  expect(decodedCall.mtime.toISOString()).toBe('2026-08-31T01:00:00.250Z')
  const stat: RunnerProtocolMessage = {
    type: 'fs_ok',
    id: 'c2',
    op: 'stat',
    result: {
      isFile: true,
      isDirectory: false,
      isSymbolicLink: false,
      mode: 0o644,
      size: 3,
      mtime: new Date(1_600_000_000_000)
    },
  }
  expect(roundTrip(stat)).toEqual(stat)

  const output: RunnerProtocolMessage = {
    type: 'spawn_output',
    spawnId: 's1',
    stream: 'stdout',
    bytes: new Uint8Array([0, 255, 10]),
  }
  const decodedOutput = roundTrip(output) as Extract<RunnerProtocolMessage, { type: 'spawn_output' }>
  expect(decodedOutput.bytes).toBeInstanceOf(Uint8Array)
  expect([...decodedOutput.bytes]).toEqual([0, 255, 10])

  // Pipes carry references only: an id and an origin-relative URL per end; the bytes go over HTTP.
  const rpcCall: RunnerProtocolMessage = {
    type: 'rpc_call',
    jobId: 'j1',
    callId: 'c9',
    root: 'demi',
    path: ['demi', 'file', 'write'],
    argv: ['demi', 'file', 'write', 'x'],
    args: { path: 'x' },
    json: false,
    cwd: '/work',
    env: {},
    stdin: true
  }
  expect(roundTrip(rpcCall)).toEqual(rpcCall)
  const cancel: RunnerProtocolMessage = { type: 'rpc_cancel', callId: 'c9' }
  expect(roundTrip(cancel)).toEqual(cancel)
  const pipes: RunnerProtocolMessage = {
    type: 'rpc_pipes',
    callId: 'c9',
    stdin: { id: 'p1', url: '/api/pipes/p1' },
    stdout: { id: 'p2', url: '/api/pipes/p2' }
  }
  expect(roundTrip(pipes)).toEqual(pipes)
  const job: RunnerProtocolMessage = {
    type: 'job_start',
    context: {
      conversation: 'test-conversation',
      caller: { kind: 'agent', node: 'test-session' },
      locale: { timeZone: 'UTC', languages: ['en-US'] }
    },
    jobId: 'j1',
    script: 'tar x',
    cwd: '/work',
    env: {},
    stdin: { id: 'p1', url: '/api/pipes/p1' }
  }
  expect(roundTrip(job)).toEqual(job)
  const done: RunnerProtocolMessage = {
    type: 'pipe_done',
    pipeId: 'p2',
    ok: false,
    error: 'pipe refused (404)'
  }
  expect(roundTrip(done)).toEqual(done)

  // Network streams name the address and the two pipes; the outcome is the
  // runner's answer under the stream id.
  const netOpen: RunnerProtocolMessage = {
    type: 'net_open',
    streamId: 'n1',
    host: '127.0.0.1',
    port: 5173,
    input: { id: 'p1', url: '/api/pipes/p1' },
    output: { id: 'p2', url: '/api/pipes/p2' }
  }
  expect(roundTrip(netOpen)).toEqual(netOpen)
  const opened: RunnerProtocolMessage = { type: 'net_opened', streamId: 'n1' }
  expect(roundTrip(opened)).toEqual(opened)
  const netError: RunnerProtocolMessage = {
    type: 'net_error',
    streamId: 'n1',
    code: 'refused',
    message: 'connection refused'
  }
  expect(roundTrip(netError)).toEqual(netError)
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({
    type: 'net_error',
    streamId: 'n1',
    code: 'no_route',
    message: 'unknown code'
  }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({
    type: 'net_open',
    streamId: 'n1',
    host: '127.0.0.1',
    port: 0,
    input: { id: 'p1', url: '/api/pipes/p1' },
    output: { id: 'p2', url: '/api/pipes/p2' }
  }))).toThrow('Malformed')

  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode(42)))
    .toThrow('Malformed')
  expect(
    () => wire.decodeBackendToRunner(msgpackCodec.encode({ no: 'type' }))
  ).toThrow('Malformed')
  expect(
    () => wire.decodeBackendToRunner(new TextEncoder()
      .encode('{"type":"ping"}'))
  ).toThrow('Malformed')
  // Validation is structural, not just type-tag: a hello without its runner
  // info, an unknown fs op, or a typed result of the wrong shape is refused.
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({
    type: 'hello',
    protocol: RUNNER_PROTOCOL_VERSION
  }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({
    type: 'fs_format_disk',
    id: 'x'
  }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({
    type: 'fs_stat',
    id: 'x'
  }))).toThrow('Malformed')
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({
    type: 'fs_ok',
    id: 'x',
    op: 'stat',
    result: 'nope'
  }))).toThrow('Malformed')
  expect(
    () => wire.decodeRunnerToBackend(msgpackCodec.encode({ type: 'pong' }))
  ).toThrow('Malformed')
})

test(
  'job hint lifetimes cross the wire independently of output and use explicit null to clear',
  () => {
    for (const hint of ['next: attending; do not poll.', null]) {
      const message = {
        type: 'job_running_hint',
        jobId: 'j1',
        invocationId: 'i1',
        hint
      } as const
      expect(wire.decodeRunnerToBackend(wire.encode(message))).toEqual(message)
    }
    for (const message of [
      { type: 'job_running_hint', jobId: 'j1', invocationId: 'i1' },
      { type: 'job_running_hint', jobId: 'j1', hint: 'hint' },
      { type: 'job_running_hint', jobId: 'j1', invocationId: 'i1', hint: 1 },
    ]) expect(
      () => wire.decodeRunnerToBackend(msgpackCodec.encode(message))
    ).toThrow('Malformed')
  }
)

test('a job carries its command context and conversation release replaces grants', () => {
  const job = { type: 'job_start', jobId: 'job', script: 'true', cwd: '/', env: {} } as const
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode(job))).toThrow('Malformed')
  const locale = { timeZone: 'UTC', languages: ['en-US'] }
  const agent = { kind: 'agent', node: 'node' } as const
  for (const context of [
    { conversation: '', caller: agent, locale },
    { conversation: 'conversation', caller: { kind: 'agent', node: '' }, locale },
    { conversation: 'conversation', caller: { kind: 'user', node: 'node' }, locale },
    { conversation: 'conversation', caller: agent, locale: { ...locale, languages: [] } },
    { conversation: 'conversation', caller: agent },
    // The shell and session ids no longer travel: the context is the identity.
    { conversation: 'conversation', caller: agent, locale, shell: 'shell' },
  ]) {
    expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ ...job, context }))).toThrow('Malformed')
  }
  const valid = { ...job, context: { conversation: 'conversation', caller: agent, locale } }
  expect(wire.decodeBackendToRunner(msgpackCodec.encode(valid))).toEqual(valid)
  const request = { type: 'conversation_release', id: 'request', conversationId: 'conversation' } as const
  expect(wire.decodeBackendToRunner(wire.encode(request))).toEqual(request)
  const response = { type: 'conversation_released', id: 'request' } as const
  expect(wire.decodeRunnerToBackend(wire.encode(response))).toEqual(response)
  for (const type of ['resource_acquire', 'resource_release', 'resource_status', 'resource_cancel']) {
    expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ type, id: 'request' }))).toThrow('Malformed')
  }
})

test('a working-tree change carries one of the pairs git status prints', () => {
  const change = { path: 'a.txt', kind: 'modified', added: 1, removed: 0 }
  for (const status of ['??', ' M', 'M ', 'AM', 'RM', 'R ', 'C ', ' A', ' T', 'T ', 'UU', 'AA', 'DD', 'UD'])
    expect(gitChangeSchema.safeParse({ ...change, status }).success).toBe(true)
  for (const status of ['  ', '!!', 'U ', ' U', ' C', 'X ', 'M', 'MMM'])
    expect(gitChangeSchema.safeParse({ ...change, status }).success).toBe(false)
})

test('a log read bounds its limit, and its lines carry their time as a Date', () => {
  const read: RunnerProtocolMessage = {
    type: 'log_read',
    id: 'l1',
    since: 41,
    limit: 200,
    source: 'service:demi.builtin'
  }
  expect(wire.decodeBackendToRunner(wire.encode(read))).toEqual(read)
  const newest: RunnerProtocolMessage = { type: 'log_read', id: 'l2', limit: 1000 }
  expect(wire.decodeBackendToRunner(wire.encode(newest))).toEqual(newest)
  for (const bad of [{ limit: 0 }, { limit: 1001 }, { limit: 10, since: -1 }, { limit: 10, source: '' }]) {
    expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({
      type: 'log_read',
      id: 'l3',
      ...bad
    }))).toThrow('Malformed')
  }

  const lines: RunnerProtocolMessage = {
    type: 'log_lines',
    id: 'l1',
    lines: [
      { at: new Date('2026-09-21T00:00:00.250Z'), source: 'runner', text: 'online' },
      {
        at: new Date('2026-09-21T00:00:01Z'),
        source: 'stream:browser.live',
        conversationId: 'conversation',
        text: 'could not list tabs'
      }
    ],
    next: 43
  }
  expect(wire.decodeRunnerToBackend(wire.encode(lines))).toEqual(lines)
  const error: RunnerProtocolMessage = { type: 'log_error', id: 'l1', message: 'permission denied' }
  expect(wire.decodeRunnerToBackend(wire.encode(error))).toEqual(error)
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({
    type: 'log_lines',
    id: 'l1',
    lines: [{ at: 'yesterday', source: 'runner', text: 'online' }],
    next: 1
  }))).toThrow('Malformed')
})
