import type { ClientFrame, ServerFrame } from './frames'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type {
  AgentTransport,
  AgentClientTransport,
  AgentServerTransport
} from './transport'

export interface JsonWebSocket {
  send(data: string): void
  close(): void
  addEventListener(
    type: 'message',
    listener: (event: { data: unknown }) => void
  ): void
  removeEventListener(
    type: 'message',
    listener: (event: { data: unknown }) => void
  ): void
}

export function createWebSocketClientTransport(
  socket: JsonWebSocket
): AgentClientTransport {
  return new WebSocketJsonTransport<ClientFrame>(socket)
}

export function createWebSocketServerTransport(
  socket: JsonWebSocket
): AgentServerTransport {
  return new WebSocketJsonTransport<ServerFrame>(socket)
}

class WebSocketJsonTransport<SendFrame> implements AgentTransport<SendFrame> {
  private readonly handlers = new Set<(frame: unknown) => void>()
  private readonly onMessage = (event: { data: unknown }): void => {
    const text = typeof event.data === 'string'
      ? event.data
      : String(event.data)
    let frame: unknown
    try {
      frame = parsePortableJson(text)
    } catch {
      // A message that is not JSON is a peer that does not speak the
      // protocol; the socket is closed rather than the message skipped.
      this.close()
      return
    }
    for (const handler of this.handlers) handler(frame)
  }

  constructor(private readonly socket: JsonWebSocket) {
    this.socket.addEventListener('message', this.onMessage)
  }

  send(frame: SendFrame): void {
    this.socket.send(stringifyPortableJson(frame))
  }

  onFrame(handler: (frame: unknown) => void): () => void {
    this.handlers.add(handler)
    return () => {
      this.handlers.delete(handler)
    }
  }

  close(): void {
    this.handlers.clear()
    this.socket.removeEventListener('message', this.onMessage)
    this.socket.close()
  }
}
