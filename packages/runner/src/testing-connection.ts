import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRunnerWire, type BackendToRunnerMessage, type RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { startRunner, type Runner } from './testing'

type Hello = Extract<RunnerToBackendMessage, { type: 'hello' }>

export interface TestConnectionOptions {
  home: string
  env?: Record<string, string>
  onHello(send: (message: BackendToRunnerMessage) => void, hello: Hello): void
  onMessage(message: RunnerToBackendMessage): void
  onClose(): void
  outgoingGate?: Promise<void>
}

/** A real Rust process behind a local test WebSocket; no model or cloud service. */
export async function connectTestRunner(options: TestConnectionOptions) {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-connection-state-'))
  const wire = createRunnerWire(msgpackCodec)
  let runner: Runner | undefined
  let socket: Bun.ServerWebSocket<unknown> | undefined
  let outgoing = options.outgoingGate ?? Promise.resolve()
  const failures: unknown[] = []
  const send = (message: BackendToRunnerMessage) => {
    const active = socket
    if (!active) throw new Error('Test runner is disconnected')
    outgoing = outgoing.then(() => {
      if (active !== socket) throw new Error('Test connection changed before send')
      active.send(wire.encode(message))
    }).catch(error => {
      failures.push(error)
      active.close()
    })
  }
  const server = Bun.serve({
    port: 0,
    fetch: (request, server) => server.upgrade(request) ? undefined : new Response('WebSocket required', { status: 400 }),
    websocket: {
      message(ws, data) {
        const message = wire.decodeRunnerToBackend(typeof data === 'string' ? new Uint8Array() : new Uint8Array(data))
        if (message.type === 'hello') {
          socket = ws
          options.onHello(send, message)
          ws.send(wire.encode({ type: 'hello_ok', deviceId: 'test-runner' }))
        } else {
          options.onMessage(message)
        }
      },
      close(ws) {
        if (socket !== ws) return
        socket = undefined
        options.onClose()
      },
    },
  })
  let phase: 'open' | 'closed' = 'open'
  const close = async () => {
    if (phase === 'closed') return
    phase = 'closed'
    await runner?.stop()
    server.stop(true)
    await rm(stateDir, { recursive: true, force: true })
    if (failures.length) throw new AggregateError(failures, 'Test connection failed')
  }
  try {
    runner = await startRunner({ backendUrl: `http://127.0.0.1:${server.port}`, stateDir, home: options.home, env: options.env, deviceToken: 'test-token' })
    return { runner, send, close }
  } catch (error) {
    await close()
    throw error
  }
}
