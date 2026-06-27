import type { ServerWebSocket } from 'bun'
import type { JsonWebSocket } from '@demicodes/agent'

type MessageListener = (event: { data: unknown }) => void

/** Adapts a Bun server WebSocket to the platform-neutral JsonWebSocket the agent transport expects. */
export class BunServerSocket implements JsonWebSocket {
  private readonly listeners = new Set<MessageListener>()

  constructor(private readonly ws: ServerWebSocket<unknown>) {}

  send(data: string): void {
    this.ws.send(data)
  }

  close(): void {
    this.ws.close()
  }

  addEventListener(_type: 'message', listener: MessageListener): void {
    this.listeners.add(listener)
  }

  removeEventListener(_type: 'message', listener: MessageListener): void {
    this.listeners.delete(listener)
  }

  receive(data: string): void {
    for (const listener of this.listeners) listener({ data })
  }
}
