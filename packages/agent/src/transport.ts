import type { ClientFrame, ServerFrame } from './frames'

export interface AgentTransport<SendFrame, ReceiveFrame> {
  send(frame: SendFrame): void
  onFrame(handler: (frame: ReceiveFrame) => void): () => void
  close(): void
}

export type AgentClientTransport = AgentTransport<ClientFrame, ServerFrame>
export type AgentServerTransport = AgentTransport<ServerFrame, ClientFrame>

export interface InProcessTransportPair {
  client: AgentClientTransport
  server: AgentServerTransport
}

export function createInProcessTransportPair(): InProcessTransportPair {
  const clientEndpoint = new InProcessEndpoint<ClientFrame, ServerFrame>()
  const serverEndpoint = new InProcessEndpoint<ServerFrame, ClientFrame>()
  clientEndpoint.connect(serverEndpoint)
  serverEndpoint.connect(clientEndpoint)
  return { client: clientEndpoint, server: serverEndpoint }
}

class InProcessEndpoint<SendFrame, ReceiveFrame> implements AgentTransport<SendFrame, ReceiveFrame> {
  private peer: InProcessEndpoint<ReceiveFrame, SendFrame> | null = null
  private readonly handlers = new Set<(frame: ReceiveFrame) => void>()
  private closed = false

  connect(peer: InProcessEndpoint<ReceiveFrame, SendFrame>): void {
    this.peer = peer
  }

  send(frame: SendFrame): void {
    if (this.closed) throw new Error('Agent transport is closed')
    if (!this.peer) throw new Error('Agent transport is not connected')
    this.peer.receive(frame)
  }

  onFrame(handler: (frame: ReceiveFrame) => void): () => void {
    this.handlers.add(handler)
    return () => {
      this.handlers.delete(handler)
    }
  }

  close(): void {
    this.closed = true
    this.handlers.clear()
  }

  private receive(frame: ReceiveFrame): void {
    if (this.closed) return
    queueMicrotask(() => {
      for (const handler of this.handlers) handler(frame)
    })
  }
}
