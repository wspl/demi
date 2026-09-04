import { expect, test } from 'bun:test'
import { RUNNER_PROTOCOL_VERSION, createRunnerWire, type RunnerProtocolMessage } from '../index'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'

const wire = createRunnerWire(msgpackCodec)

test('runner messages round-trip through the MessagePack wire', () => {
  const runnerToBackend = new Set(['hello', 'pong', 'fs_ok', 'fs_error', 'spawn_output', 'spawn_exit', 'rpc_call', 'rpc_cancel', 'pipe_done'])
  const roundTrip = (message: RunnerProtocolMessage): RunnerProtocolMessage =>
    runnerToBackend.has(message.type) ? wire.decodeRunnerToBackend(wire.encode(message)) : wire.decodeBackendToRunner(wire.encode(message))

  const hello: RunnerProtocolMessage = {
    type: 'hello',
    protocol: RUNNER_PROTOCOL_VERSION,
    deviceToken: 'token',
    runner: { name: 'dev-box', platform: 'darwin', version: '1.0.0', identity: { uid: 501, gid: 20, hostname: 'mac', homeDir: '/Users/dev' } },
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
    result: { isFile: true, isDirectory: false, isSymbolicLink: false, mode: 0o644, size: 3, mtime: new Date(1_600_000_000_000) },
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
  const rpcCall: RunnerProtocolMessage = { type: 'rpc_call', callId: 'c9', agentSessionId: 'a', shellId: 's', root: 'demi', path: ['demi', 'file', 'write'], argv: ['demi', 'file', 'write', 'x'], args: { path: 'x' }, json: false, cwd: '/work', env: {}, stdin: true }
  expect(roundTrip(rpcCall)).toEqual(rpcCall)
  const cancel: RunnerProtocolMessage = { type: 'rpc_cancel', callId: 'c9' }
  expect(roundTrip(cancel)).toEqual(cancel)
  const pipes: RunnerProtocolMessage = { type: 'rpc_pipes', callId: 'c9', stdin: { id: 'p1', url: '/api/pipes/p1' }, stdout: { id: 'p2', url: '/api/pipes/p2' } }
  expect(roundTrip(pipes)).toEqual(pipes)
  const job: RunnerProtocolMessage = { type: 'job_start', jobId: 'j1', script: 'tar x', cwd: '/work', env: {}, stdin: { id: 'p1', url: '/api/pipes/p1' } }
  expect(roundTrip(job)).toEqual(job)
  const done: RunnerProtocolMessage = { type: 'pipe_done', pipeId: 'p2', ok: false, error: 'pipe refused (404)' }
  expect(roundTrip(done)).toEqual(done)

  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode(42))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ no: 'type' }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(new TextEncoder().encode('{"type":"ping"}'))).toThrow('Malformed')
  // Validation is structural, not just type-tag: a hello without its runner
  // info, an unknown fs op, or a typed result of the wrong shape is refused.
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({ type: 'hello', protocol: RUNNER_PROTOCOL_VERSION }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ type: 'fs_format_disk', id: 'x' }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ type: 'fs_stat', id: 'x' }))).toThrow('Malformed')
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({ type: 'fs_ok', id: 'x', op: 'stat', result: 'nope' }))).toThrow('Malformed')
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({ type: 'pong' }))).toThrow('Malformed')
})
