import { mkdtemp } from 'node:fs/promises'
import { existsSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentServer, type AgentHarness, type ClientSessionEvent } from '@demicodes/agent'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { RemoteHost, RemoteShellEnvironment } from '@demicodes/host-remote'
import { createRunnerWire, type BackendToRunnerMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { memoryHostStore } from '@demicodes/shell/testing'
import { waitFor } from '@demicodes/utils'
import { startTinyjsRunner } from '../testing'

// M1 acceptance on the tinyjs runner: a bare AgentServer executing on a real
// runner process over a real WebSocket — commands run as jobs in the
// runner's home, a runner death mid-command surfaces as an ordinary tool
// error without losing the session, and after reconnect the next command
// succeeds.

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}
const selection = { providerId: 'stub', model }
const wire = createRunnerWire(msgpackCodec)

test('bare AgentServer executes over a live runner; death mid-command is a tool error; reconnect resumes', async () => {
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-runner-e2e-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-runner-state-'))
  const remoteHost = new RemoteHost({
    defaultCwd: runnerDir,
    identity: new LocalHost(runnerDir).identity,
    store: memoryHostStore(),
  })

  // Bare backend socket: accept the runner, reply hello_ok, and bind the
  // connection to the RemoteHost. No claim flow — that is M2 product surface.
  type SocketData = { authed: boolean }
  const active: { socket: Bun.ServerWebSocket<SocketData> | null } = { socket: null }
  /** Every frame type the runner sent, in order: the wire audit of the acceptance. */
  const inbound: string[] = []
  const server = Bun.serve<SocketData>({
    port: 0,
    fetch(request, bunServer) {
      if (bunServer.upgrade(request, { data: { authed: false } })) return
      return new Response('runner endpoint', { status: 400 })
    },
    websocket: {
      message(ws, data) {
        if (typeof data === 'string') throw new Error('runner frames are binary')
        const message = wire.decodeRunnerToBackend(new Uint8Array(data))
        inbound.push(message.type)
        if (message.type === 'hello') {
          active.socket = ws
          ws.data.authed = true
          remoteHost.attach((outgoing: BackendToRunnerMessage) => {
            ws.send(wire.encode(outgoing))
          })
          ws.send(wire.encode({ type: 'hello_ok', deviceId: 'device-test' }))
          return
        }
        remoteHost.handleMessage(message)
      },
      close(ws) {
        if (active.socket === ws) {
          active.socket = null
          remoteHost.detach('runner disconnected')
        }
      },
    },
  })

  const runner = await startTinyjsRunner({ backendUrl: `ws://localhost:${server.port}`, stateDir, home: runnerDir, name: 'test-runner' })
  await waitFor(() => remoteHost.online, () => runner.log.join('\n'), { timeoutMs: 10_000 })

  const harness: AgentHarness<Record<string, never>> = {
    name: 'runner-e2e-test',
    initialState: () => ({}),
    host: () => remoteHost,
    systemPrompt: () => 'test',
  }
  const provider = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () =>
      new StubProvider([
        // Turn 1: real cat/tee spawns on the runner, files land in its temp dir.
        [events.toolCall('t1', 'shell_exec', { script: 'printf hello | tee made.txt | cat', timeoutMs: 10_000 })],
        [events.text('turn one done'), events.response()],
        // Turn 2: killed mid-command by dropping the runner connection.
        [events.toolCall('t2', 'shell_exec', { script: 'echo started && sleep 30', timeoutMs: 60_000 })],
        [events.text('survived the drop'), events.response()],
        // Turn 3: after reconnect the same Host serves again.
        [events.toolCall('t3', 'shell_exec', { script: 'cat made.txt', timeoutMs: 10_000 })],
        [events.text('turn three done'), events.response()],
      ]),
  })

  const agentServer = new AgentServer({
    agent: harness,
    providers: [provider],
    shellEnvironment: (ctx) => new RemoteShellEnvironment({ ...ctx.shell, host: remoteHost }),
  })
  const client = agentServer.client()
  const shellEvents: Extract<ClientSessionEvent, { type: 'shell_output' }>[] = []
  client.subscribe((event) => {
    if (event.type === 'shell_output') shellEvents.push(event)
  })
  await client.open(selection, runnerDir, globalThis.crypto.randomUUID())

  // Turn 1: executes on the runner machine (the file really exists there),
  // and `cmd1 | cmd2` is an OS pipe there: the wire saw the job's view and
  // its exit, no spawn and no file bytes.
  const framesBefore = inbound.length
  await client.send([{ type: 'text', text: 'run on the runner' }])
  const turn1 = shellEvents.filter((event) => event.status.status === 'exited')
  expect(turn1.at(-1)?.status.stdout.delta).toBe('hello')
  expect(existsSync(join(runnerDir, 'made.txt'))).toBe(true)
  expect(readFileSync(join(runnerDir, 'made.txt'), 'utf8')).toBe('hello')
  expect(new Set(inbound.slice(framesBefore))).toEqual(new Set(['job_output', 'job_exit']))

  // Turn 2: drop the runner while `sleep 30` runs; the command dies as an
  // ordinary tool error and the turn still completes.
  const eventsBeforeTurn2 = shellEvents.length
  const sendPromise = client.send([{ type: 'text', text: 'now hang' }])
  await waitFor(() => remoteHost.activeJobCount > 0, undefined, { timeoutMs: 10_000 })
  active.socket?.close()
  await sendPromise
  const failed = shellEvents.slice(eventsBeforeTurn2).at(-1)
  expect(failed).toBeDefined()
  expect(failed?.status.status).not.toBe('running')

  // Reconnect happens on its own (backoff); the same RemoteHost object goes
  // back online, so the session's shell state and Host identity are intact.
  await waitFor(() => remoteHost.online, () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const eventsBeforeTurn3 = shellEvents.length
  await client.send([{ type: 'text', text: 'read it back' }])
  const turn3 = shellEvents.slice(eventsBeforeTurn3).at(-1)
  expect(turn3?.status.status).toBe('exited')
  expect(turn3?.status.stdout.delta).toBe('hello')

  await client.close()
  await agentServer.close()
  await runner.stop()
  server.stop(true)
}, 60_000)
