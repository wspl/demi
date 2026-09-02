import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { LocalHost } from '@demicodes/host-local'
import {
  HostRpcServer,
  RemoteHost,
  createRunnerWire,
  msgpackCodec,
  type BackendToRunnerMessage,
  type RunnerProtocolMessage,
  type RunnerToBackendMessage,
} from '../index'
import { memoryHostStore } from '@demicodes/shell/testing'

const wire = createRunnerWire(msgpackCodec)

test('runner messages round-trip through the MessagePack wire', () => {
  const runnerToBackend = new Set(['hello', 'pong', 'fs_ok', 'fs_error', 'spawn_output', 'spawn_exit'])
  const roundTrip = (message: RunnerProtocolMessage): RunnerProtocolMessage =>
    runnerToBackend.has(message.type) ? wire.decodeRunnerToBackend(wire.encode(message)) : wire.decodeBackendToRunner(wire.encode(message))

  const hello: RunnerProtocolMessage = {
    type: 'hello',
    protocol: 2,
    deviceToken: 'token',
    runner: { name: 'dev-box', platform: 'darwin', version: '1.0.0', identity: { uid: 501, gid: 20, hostname: 'mac' } },
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

  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode(42))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ no: 'type' }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(new TextEncoder().encode('{"type":"ping"}'))).toThrow('Malformed')
  // Validation is structural, not just type-tag: a hello without its runner
  // info, an unknown fs op, or a typed result of the wrong shape is refused.
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({ type: 'hello', protocol: 2 }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ type: 'fs_format_disk', id: 'x' }))).toThrow('Malformed')
  expect(() => wire.decodeBackendToRunner(msgpackCodec.encode({ type: 'fs_stat', id: 'x' }))).toThrow('Malformed')
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({ type: 'fs_ok', id: 'x', op: 'stat', result: 'nope' }))).toThrow('Malformed')
  expect(() => wire.decodeRunnerToBackend(msgpackCodec.encode({ type: 'pong' }))).toThrow('Malformed')
})

/** RemoteHost and HostRpcServer joined directly (encoded through the codec both ways). */
async function connectedPair() {
  const dir = await mkdtemp(join(tmpdir(), 'demi-runner-proto-'))
  const local = new LocalHost(dir)
  const remote = new RemoteHost({
    defaultCwd: dir,
    commandArtifactsDir: join(dir, '.artifacts'),
    identity: { uid: 501, gid: 20, hostname: 'test' },
    store: memoryHostStore(),
  })
  const server = new HostRpcServer(local, (message: RunnerToBackendMessage) => {
    remote.handleMessage(wire.decodeRunnerToBackend(wire.encode(message)))
  })
  remote.attach((message: BackendToRunnerMessage) => {
    void server.handleMessage(wire.decodeBackendToRunner(wire.encode(message)))
  })
  return { dir, remote, server }
}

test('remote fs calls execute on the served Host and preserve error codes', async () => {
  const { dir, remote } = await connectedPair()

  await remote.fs.writeFile(join(dir, 'hello.txt'), new TextEncoder().encode('hi'), { createParents: true })
  expect(new TextDecoder().decode(await remote.fs.readFile(join(dir, 'hello.txt')))).toBe('hi')

  const stat = await remote.fs.stat(join(dir, 'hello.txt'))
  expect(stat.isFile).toBe(true)
  expect(stat.size).toBe(2)
  expect(stat.mtime).toBeInstanceOf(Date)

  await remote.fs.mkdir(join(dir, 'sub/deeper'), { recursive: true })
  const names = await remote.fs.readdir(dir)
  expect(names.sort()).toEqual(['hello.txt', 'sub'])
  const entries = await remote.fs.readdir(dir, { withFileTypes: true })
  expect(entries.find((entry) => entry.name === 'sub')?.isDirectory).toBe(true)

  expect(await remote.fs.exists(join(dir, 'nope'))).toBe(false)
  const missing = await remote.fs.readFile(join(dir, 'nope')).catch((error: NodeJS.ErrnoException) => error)
  expect(missing).toBeInstanceOf(Error)
  expect((missing as NodeJS.ErrnoException).code).toBe('ENOENT')
})

test('remote spawn streams output, accepts stdin, and reports exit', async () => {
  const { dir, remote } = await connectedPair()

  const echo = await remote.process.spawn({ command: '/bin/echo', args: ['over the wire'], cwd: dir, env: { PATH: '/usr/bin:/bin' } })
  expect(await collect(echo.stdout)).toBe('over the wire\n')
  expect((await echo.wait()).exitCode).toBe(0)

  const cat = await remote.process.spawn({ command: '/bin/cat', cwd: dir, env: { PATH: '/usr/bin:/bin' } })
  await cat.writeStdin(new TextEncoder().encode('stdin data'))
  await cat.closeStdin()
  expect(await collect(cat.stdout)).toBe('stdin data')
  expect((await cat.wait()).exitCode).toBe(0)

  const sleeper = await remote.process.spawn({ command: '/bin/sleep', args: ['30'], cwd: dir, env: { PATH: '/usr/bin:/bin' } })
  await sleeper.kill('SIGTERM')
  const exit = await sleeper.wait()
  expect(exit.exitCode).not.toBe(0)

  const missing = await remote.process.spawn({ command: 'not-a-real-binary', cwd: dir, env: { PATH: '/usr/bin:/bin' } })
  expect((await missing.wait()).spawnError?.kind).toBe('executable_not_found')
})

test('detach fails pending calls and kills in-flight spawn views; reattach resumes', async () => {
  const { dir, remote, server } = await connectedPair()

  const running = await remote.process.spawn({ command: '/bin/sleep', args: ['30'], cwd: dir, env: { PATH: '/usr/bin:/bin' } })
  remote.detach('runner disconnected')
  const exit = await running.wait()
  expect(exit.exitCode).toBeNull()
  expect(exit.spawnError?.kind).toBe('other')
  await server.close() // the real connection-drop path also kills runner-side children

  await expect(remote.fs.readFile(join(dir, 'x'))).rejects.toThrow('runner disconnected')
  const offlineSpawn = await remote.process.spawn({ command: '/bin/echo', cwd: dir })
  expect((await offlineSpawn.wait()).spawnError?.kind).toBe('other')

  // Reattach: the same Host object serves again.
  const server2 = new HostRpcServer(new LocalHost(dir), (message) => remote.handleMessage(message))
  remote.attach((message) => {
    void server2.handleMessage(message)
  })
  await remote.fs.writeFile(join(dir, 'back.txt'), new TextEncoder().encode('online'))
  expect(new TextDecoder().decode(await remote.fs.readFile(join(dir, 'back.txt')))).toBe('online')
})

test('logical cwd and identity are backend-local', async () => {
  const { dir, remote } = await connectedPair()
  const cwd = await remote.process.openCwd(dir)
  await cwd.chdir('sub')
  expect(cwd.path).toBe(join(dir, 'sub'))
  const snapshot = await cwd.snapshot()
  await cwd.chdir('..')
  snapshot.restore()
  expect(cwd.path).toBe(join(dir, 'sub'))
  await cwd.close()
  expect(remote.identity.hostname).toBe('test')
})

async function collect(stream: AsyncIterable<Uint8Array>): Promise<string> {
  let text = ''
  const decoder = new TextDecoder()
  for await (const chunk of stream) text += decoder.decode(chunk, { stream: true })
  return text + decoder.decode()
}
