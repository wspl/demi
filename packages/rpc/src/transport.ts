import type { ClientFrame, ServerFrame } from './frames'

export interface RPCTransport<SendFrame, ReceiveFrame> {
  send(frame: SendFrame): void
  onFrame(handler: (frame: ReceiveFrame) => void): () => void
  close(): void
}

export type RpcClientTransport = RPCTransport<ClientFrame, ServerFrame>
export type RpcHostTransport = RPCTransport<ServerFrame, ClientFrame>

export interface InProcessTransportPair {
  client: RpcClientTransport
  host: RpcHostTransport
}

export function createInProcessTransportPair(): InProcessTransportPair {
  const clientEndpoint = new InProcessEndpoint<ClientFrame, ServerFrame>()
  const hostEndpoint = new InProcessEndpoint<ServerFrame, ClientFrame>()
  clientEndpoint.connect(hostEndpoint)
  hostEndpoint.connect(clientEndpoint)
  return { client: clientEndpoint, host: hostEndpoint }
}

class InProcessEndpoint<SendFrame, ReceiveFrame> implements RPCTransport<SendFrame, ReceiveFrame> {
  private peer: InProcessEndpoint<ReceiveFrame, SendFrame> | null = null
  private readonly handlers = new Set<(frame: ReceiveFrame) => void>()
  private closed = false

  connect(peer: InProcessEndpoint<ReceiveFrame, SendFrame>): void {
    this.peer = peer
  }

  send(frame: SendFrame): void {
    if (this.closed) throw new Error('RPC transport is closed')
    if (!this.peer) throw new Error('RPC transport is not connected')
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
