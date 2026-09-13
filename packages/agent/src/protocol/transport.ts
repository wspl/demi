import type { ClientFrame, ServerFrame } from './frames'

/**
 * A transport carries frames; it does not vouch for them. What arrives is
 * handed over as `unknown`, and the endpoint that receives it (the server
 * binding, the client) validates it against its frame schema.
 */
export interface AgentTransport<SendFrame> {
  send(frame: SendFrame): void
  /**
   * Async completion lets transport adapters hold admission until frame
   * handling finishes.
   */
  onFrame(handler: (frame: unknown) => void | Promise<void>): () => void
  close(): void
}

export type AgentClientTransport = AgentTransport<ClientFrame>
export type AgentServerTransport = AgentTransport<ServerFrame>

export interface InProcessTransportPair {
  client: AgentClientTransport
  server: AgentServerTransport
}

export function createInProcessTransportPair(): InProcessTransportPair {
  const clientEndpoint = new InProcessEndpoint<ClientFrame>()
  const serverEndpoint = new InProcessEndpoint<ServerFrame>()
  clientEndpoint.connect(serverEndpoint)
  serverEndpoint.connect(clientEndpoint)
  return { client: clientEndpoint, server: serverEndpoint }
}

class InProcessEndpoint<SendFrame> implements AgentTransport<SendFrame> {
  private peer: InProcessEndpoint<unknown> | null = null
  private readonly handlers = new Set<(frame: unknown) => void>()
  private closed = false

  connect(peer: InProcessEndpoint<unknown>): void {
    this.peer = peer
  }

  send(frame: SendFrame): void {
    if (this.closed)
      throw new Error('Agent transport is closed')
    if (!this.peer)
      throw new Error('Agent transport is not connected')
    this.peer.receive(frame)
  }

  onFrame(handler: (frame: unknown) => void | Promise<void>): () => void {
    this.handlers.add(handler)
    return () => {
      this.handlers.delete(handler)
    }
  }

  close(): void {
    this.closed = true
    this.handlers.clear()
  }

  private receive(frame: unknown): void {
    if (this.closed)
      return
    queueMicrotask(() => {
      for (const handler of this.handlers) handler(frame)
    })
  }
}
