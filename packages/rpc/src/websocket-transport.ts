import type { ClientFrame, ServerFrame } from './frames'
import { parseRpcJson, stringifyRpcJson } from './json-codec'
import type { RPCTransport, RpcClientTransport, RpcHostTransport } from './transport'

export interface JsonWebSocket {
  send(data: string): void
  close(): void
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void
  removeEventListener(type: 'message', listener: (event: { data: unknown }) => void): void
}

export function createWebSocketClientTransport(socket: JsonWebSocket): RpcClientTransport {
  return new WebSocketJsonTransport<ClientFrame, ServerFrame>(socket)
}

export function createWebSocketHostTransport(socket: JsonWebSocket): RpcHostTransport {
  return new WebSocketJsonTransport<ServerFrame, ClientFrame>(socket)
}

class WebSocketJsonTransport<SendFrame, ReceiveFrame> implements RPCTransport<SendFrame, ReceiveFrame> {
  private readonly handlers = new Set<(frame: ReceiveFrame) => void>()
  private readonly onMessage = (event: { data: unknown }): void => {
    const text = typeof event.data === 'string' ? event.data : String(event.data)
    const frame = parseRpcJson<ReceiveFrame>(text)
    for (const handler of this.handlers) handler(frame)
  }

  constructor(private readonly socket: JsonWebSocket) {
    this.socket.addEventListener('message', this.onMessage)
  }

  send(frame: SendFrame): void {
    this.socket.send(stringifyRpcJson(frame))
  }

  onFrame(handler: (frame: ReceiveFrame) => void): () => void {
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
