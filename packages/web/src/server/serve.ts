import process from 'node:process'
import type { AgentTransportBinding } from '@demi/agent'
import type { Provider } from '@demi/provider'
import type { ControlMethod } from '@demi/web-ui/transport/protocol'
import { AgentHub } from './agent-hub'
import { BunServerSocket } from './bun-socket'
import { ControlServer } from './control-server'
import type { ServerOptions } from './server-options'

type AgentConn = { kind: 'agent'; cwd: string; socket?: BunServerSocket; binding?: AgentTransportBinding }
type ConnData = AgentConn | { kind: 'control' }

const BACKEND_ONLY_MESSAGE = 'Demi web backend only exposes /control and /agent. Open the Vite dev server for the browser UI.'

export interface WebServerHandle {
  port: number
  url: string
  stop(): Promise<void>
}

export function startWebServer(providers: Provider[], options: ServerOptions): WebServerHandle {
  const hub = new AgentHub(providers, {
    initialEnv: { PATH: process.env.PATH ?? '' },
    yieldAfterMs: options.yieldAfterMs,
  })
  const control = new ControlServer(providers, options)

  const server = Bun.serve<ConnData>({
    port: options.port,
    async fetch(req, srv) {
      const url = new URL(req.url)
      if (url.pathname === '/agent') {
        const cwd = url.searchParams.get('cwd') ?? options.cwd
        if (srv.upgrade(req, { data: { kind: 'agent', cwd } })) return undefined
        return new Response('Upgrade failed', { status: 426 })
      }
      if (url.pathname === '/control') {
        if (srv.upgrade(req, { data: { kind: 'control' } })) return undefined
        return new Response('Upgrade failed', { status: 426 })
      }
      return new Response(BACKEND_ONLY_MESSAGE, {
        status: 404,
        headers: { 'content-type': 'text/plain; charset=utf-8' },
      })
    },
    websocket: {
      open(ws) {
        const data = ws.data
        if (data.kind !== 'agent') return
        const socket = new BunServerSocket(ws)
        data.socket = socket
        data.binding = hub.attach(data.cwd, socket)
      },
      message(ws, message) {
        const data = ws.data
        const text = typeof message === 'string' ? message : message.toString()
        if (data.kind === 'agent') {
          data.socket?.receive(text)
          return
        }
        void replyControl(control, ws, text)
      },
      close(ws) {
        const data = ws.data
        if (data.kind === 'agent') void data.binding?.close()
      },
    },
  })

  const port = server.port ?? options.port
  return {
    port,
    url: `http://localhost:${port}`,
    async stop() {
      server.stop(true)
      await hub.close()
    },
  }
}

async function replyControl(
  control: ControlServer,
  ws: { send(data: string): void },
  text: string,
): Promise<void> {
  let id = 0
  try {
    const request = JSON.parse(text) as { id: number; method: ControlMethod; params: unknown }
    id = request.id
    const result = await control.handle(request.method, request.params)
    ws.send(JSON.stringify({ id, ok: true, result }))
  } catch (error) {
    ws.send(JSON.stringify({ id, ok: false, error: error instanceof Error ? error.message : String(error) }))
  }
}
