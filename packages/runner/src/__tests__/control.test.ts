import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRunnerWire, type BackendToRunnerMessage, type RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { waitFor } from '@demicodes/utils'
import { startTinyjsRunner } from '../testing'

const wire = createRunnerWire(msgpackCodec)

test('blocked job and spawn stdin leave ping, filesystem and kill responsive; stdin order survives EOF', async () => {
  const home = await mkdtemp(join(tmpdir(), 'demi-control-home-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-control-state-'))
  const received: RunnerToBackendMessage[] = []
  let socket!: Bun.ServerWebSocket<unknown>
  const server = Bun.serve({ port: 0,
    fetch(request, server) { return server.upgrade(request) ? undefined : new Response('bad', { status: 400 }) },
    websocket: { message(ws, data) {
      const message = wire.decodeRunnerToBackend(typeof data === 'string' ? new Uint8Array() : new Uint8Array(data))
      received.push(message)
      if (message.type === 'hello') {
        socket = ws
        ws.send(wire.encode({ type: 'hello_ok', deviceId: 'device' }))
      }
    } },
  })
  const runner = await startTinyjsRunner({ backendUrl: `http://localhost:${server.port}`, home, stateDir, deviceToken: 'test' })
  const send = (message: BackendToRunnerMessage) => socket.send(wire.encode(message))
  try {
    await waitFor(() => runner.statuses.includes('online'))
    for (const kind of ['job', 'spawn'] as const) {
      const start = received.length
      const script = 'printf ready; sleep 2'
      send(kind === 'job'
        ? { type: 'job_start', jobId: kind, script, cwd: home, env: {} }
        : { type: 'spawn', spawnId: kind, command: 'bash', args: ['-c', script], killProcessGroup: true })
      await waitFor(() => received.slice(start).some((message) => message.type === `${kind}_output`))
      const bytes = new Uint8Array(256 * 1024)
      send(kind === 'job' ? { type: 'job_stdin', jobId: kind, bytes } : { type: 'spawn_stdin', spawnId: kind, bytes })
      send({ type: 'ping' })
      send({ type: 'fs_stat', id: kind, path: home })
      send(kind === 'job' ? { type: 'job_kill', jobId: kind, signal: 'SIGKILL' } : { type: 'spawn_kill', spawnId: kind, signal: 'SIGKILL' })
      await waitFor(() => received.slice(start).some((message) => message.type === 'pong') && received.some((message) => message.type === 'fs_ok' && message.id === kind), () => runner.log.join('\n'), { timeoutMs: 750 })
      await waitFor(() => received.slice(start).some((message) => message.type === `${kind}_exit`))
      expect(received.slice(start).find((message) => message.type === `${kind}_exit`)).toMatchObject({ signal: 'SIGKILL' })
    }
    send({ type: 'spawn', spawnId: 'ordered', command: 'cat' })
    send({ type: 'spawn_stdin', spawnId: 'ordered', bytes: new TextEncoder().encode('first\n') })
    send({ type: 'spawn_stdin', spawnId: 'ordered', bytes: new TextEncoder().encode('second\n') })
    send({ type: 'spawn_stdin_end', spawnId: 'ordered' })
    await waitFor(() => received.some((message) => message.type === 'spawn_exit' && message.spawnId === 'ordered'))
    const output = received.flatMap((message) => message.type === 'spawn_output' && message.spawnId === 'ordered' ? [message.bytes] : [])
    expect(await new Blob(output).text()).toBe('first\nsecond\n')
  } finally {
    await runner.stop()
    server.stop(true)
  }
}, 15_000)
