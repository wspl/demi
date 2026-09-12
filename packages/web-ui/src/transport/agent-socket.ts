import {
  AgentClient,
  type ClientFrame,
  createWebSocketTransport,
  displayedServerFrameSchema,
} from '@demicodes/agent/client'
import type { AgentClient as DisplayedAgentClient, ServerFrame } from './protocol'

export function agentSocketUrl(baseUrl: string, cwd: string): string {
  const url = new URL('/agent', baseUrl)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  url.searchParams.set('cwd', cwd)
  return url.toString()
}

/**
 * The connection itself could not be made or was lost before it opened: a
 * transport failure, not the session's. The runtime retries these on its own.
 */
export class AgentSocketError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'AgentSocketError'
  }
}

export interface AgentSocketOptions {
  signal?: AbortSignal
  /** Product wire extensions are translated before serialization. */
  encodeFrame?: (frame: ClientFrame) => unknown
  decodeFrame?: (frame: unknown) => ServerFrame
}

/** A socket lifetime is a connection lifetime, never a server-task lifetime. */
export function connectAgentClient(
  url: string,
  options: AgentSocketOptions = {},
): Promise<DisplayedAgentClient> {
  return new Promise((resolve, reject) => {
    options.signal?.throwIfAborted()
    const socket = new WebSocket(url)
    let client: DisplayedAgentClient | null = null
    const cleanup = () => {
      clearTimeout(timeout)
      socket.removeEventListener('open', opened)
      socket.removeEventListener('error', failed)
      socket.removeEventListener('close', closed)
      options.signal?.removeEventListener('abort', aborted)
    }
    const detach = () => {
      cleanup()
      socket.close()
    }
    const closed = () => {
      cleanup()
      if (client) {
        client.disconnect()
      } else {
        reject(new AgentSocketError('Agent socket closed before opening'))
      }
    }
    const failed = () => {
      const error = new AgentSocketError('Agent socket failed to connect')
      if (client) {
        client.disconnect(error)
      } else {
        detach()
        reject(error)
      }
    }
    const aborted = () => {
      if (client) {
        client.disconnect()
      } else {
        detach()
        reject(options.signal?.reason)
      }
    }
    const opened = () => {
      clearTimeout(timeout)
      socket.removeEventListener('open', opened)
      socket.removeEventListener('error', failed)
      socket.removeEventListener('close', closed)
      const transport = createWebSocketTransport<ClientFrame, ServerFrame>(socket, {
        decode: options.decodeFrame ?? displayedServerFrameSchema.parse,
        encode: options.encodeFrame,
      })
      client = new AgentClient({
        send: (frame) => transport.send(frame),
        onFrame: (handler) => transport.onFrame(handler),
        onError: (handler) => transport.onError(handler),
        close: () => {
          cleanup()
          transport.close()
        },
      })
      resolve(client)
    }
    const timeout = setTimeout(failed, 15_000)
    socket.addEventListener('open', opened)
    socket.addEventListener('error', failed)
    socket.addEventListener('close', closed)
    options.signal?.addEventListener('abort', aborted, { once: true })
  })
}
