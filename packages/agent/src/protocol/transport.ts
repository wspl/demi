import type { Block } from '@demicodes/core'
import type { ClientFrame, ServerFrame } from './frames'
import { AgentTransportError, FrameChannel } from './frame-channel'
export { AgentTransportError } from './frame-channel'

export interface AgentTransport<SendFrame, ReceiveFrame> {
  send(frame: SendFrame): void
  /**
   * Async completion lets transport adapters hold admission until frame
   * handling finishes.
   */
  onFrame(handler: (frame: ReceiveFrame) => void | Promise<void>): () => void
  onError(handler: (error: AgentTransportError) => void): () => void
  close(): void
}

export type AgentClientTransport<B extends Block<unknown, unknown> = Block> = AgentTransport<ClientFrame, ServerFrame<B>>
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

class InProcessEndpoint<SendFrame, ReceiveFrame>
  implements AgentTransport<SendFrame, ReceiveFrame> {
  private peer: InProcessEndpoint<ReceiveFrame, SendFrame> | null = null
  private readonly channel = new FrameChannel<ReceiveFrame>()
  private closed = false

  connect(peer: InProcessEndpoint<ReceiveFrame, SendFrame>): void {
    this.peer = peer
  }

  send(frame: SendFrame): void {
    if (this.closed)
      throw new Error('Agent transport is closed')
    if (!this.peer)
      throw new Error('Agent transport is not connected')
    this.peer.receive(frame)
  }

  onFrame(handler: (frame: ReceiveFrame) => void | Promise<void>): () => void {
    return this.channel.onFrame(handler)
  }

  onError(handler: (error: AgentTransportError) => void): () => void {
    return this.channel.onError(handler)
  }

  close(): void {
    if (this.closed)
      return
    this.closed = true
    this.channel.close()
    this.peer?.channel.report(new AgentTransportError('closed', 'Agent transport closed'))
    this.peer = null
  }

  private receive(frame: ReceiveFrame): void {
    if (!this.closed)
      void this.channel.receive(frame)
  }
}
