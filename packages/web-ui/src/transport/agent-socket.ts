import {
  AgentClient,
  type ClientFrame,
  type ServerFrame,
} from '@demicodes/agent/client'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'

export function agentSocketUrl(baseUrl: string, cwd: string): string {
  const url = new URL('/agent', baseUrl)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  url.searchParams.set('cwd', cwd)
  return url.toString()
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
): Promise<AgentClient> {
  return new Promise((resolve, reject) => {
    options.signal?.throwIfAborted()
    const socket = new WebSocket(url)
    let client: AgentClient | null = null
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
        reject(new Error('Agent socket closed before opening'))
      }
    }
    const failed = () => {
      const error = new Error('Agent socket failed to connect')
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
      const listeners = new Set<(event: MessageEvent) => void>()
      client = new AgentClient({
        send: (frame) => {
          const encoded = options.encodeFrame ? options.encodeFrame(frame) : frame
          socket.send(stringifyPortableJson(encoded))
        },
        onFrame: (handler) => {
          const listener = (event: MessageEvent) => {
            try {
              const frame = parsePortableJson<ServerFrame>(String(event.data))
              handler(options.decodeFrame ? options.decodeFrame(frame) : frame)
            } catch (error) {
              client?.disconnect(
                error instanceof Error ? error : new Error(String(error)),
              )
            }
          }
          listeners.add(listener)
          socket.addEventListener('message', listener)
          return () => {
            listeners.delete(listener)
            socket.removeEventListener('message', listener)
          }
        },
        close: () => {
          for (const listener of listeners) {
            socket.removeEventListener('message', listener)
          }
          listeners.clear()
          detach()
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
