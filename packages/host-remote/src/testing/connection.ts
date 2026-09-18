import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRunnerWire, type BackendToRunnerMessage, type RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import type { PipeBroker } from '../pipes'
import { startRunner, type Runner } from '../testing'

type Hello = Extract<RunnerToBackendMessage, { type: 'hello' }>

/** The device id the bridge gives its runner, and the one its pipes are bound to. */
export const TEST_RUNNER_DEVICE = 'test-runner'
const TEST_RUNNER_TOKEN = 'test-token'

export interface TestConnectionOptions {
  home: string
  env?: Record<string, string>
  /**
   * The broker whose device ends the runner reaches at `/api/pipes/:id`, as
   * the backend's pipe routes serve them; a Host's file contents need it.
   */
  pipes?: PipeBroker
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
    // As the backend's registry does: a message over the limit throws to the
    // request it belongs to, and the connection carries on.
    const frame = wire.encode(message)
    outgoing = outgoing.then(() => {
      if (active !== socket) throw new Error('Test connection changed before send')
      active.send(frame)
    }).catch(error => {
      failures.push(error)
      active.close()
    })
  }
  const server = Bun.serve({
    port: 0,
    fetch: async (request, server) => {
      if (options.pipes && pipeId(request) !== null)
        return servePipe(options.pipes, request, { id: TEST_RUNNER_DEVICE, token: TEST_RUNNER_TOKEN })
      return server.upgrade(request) ? undefined : new Response('WebSocket required', { status: 400 })
    },
    websocket: {
      message(ws, data) {
        const message = wire.decodeRunnerToBackend(typeof data === 'string' ? new Uint8Array() : new Uint8Array(data))
        if (message.type === 'hello') {
          socket = ws
          options.onHello(send, message)
          ws.send(wire.encode({ type: 'hello_ok', deviceId: TEST_RUNNER_DEVICE }))
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
    runner = await startRunner({ backendUrl: `http://127.0.0.1:${server.port}`, stateDir, home: options.home, env: options.env, deviceToken: TEST_RUNNER_TOKEN })
    return { runner, send, close }
  } catch (error) {
    await close()
    throw error
  }
}

/** The pipe a request to `/api/pipes/:id` names, or null for any other path. */
export function pipeId(request: Request): string | null {
  return new URL(request.url).pathname.match(/^\/api\/pipes\/([^/]+)$/)?.[1] ?? null
}

/**
 * One device end of a pipe, the way the backend's pipe routes answer it: the
 * runner authenticated by `device.token` is `device.id` to the broker.
 */
export async function servePipe(
  broker: PipeBroker,
  request: Request,
  device: { id: string; token: string },
): Promise<Response> {
  const id = pipeId(request)
  if (id === null)
    return new Response('not a pipe', { status: 404 })
  if (request.headers.get('authorization') !== `Bearer ${device.token}`)
    return new Response('device token required', { status: 401 })
  if (request.method === 'PUT') {
    const result = await broker.put(id, device.id, request.body, request.signal)
    return new Response(result.message, { status: result.status })
  }
  const result = await broker.get(id, device.id, request.signal)
  if (!('body' in result))
    return new Response(result.message, { status: result.status })
  return new Response(result.body, { status: 200 })
}
