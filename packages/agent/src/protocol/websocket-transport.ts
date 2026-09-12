import type { ClientFrame, ServerFrame } from './frames'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { AgentTransport, AgentClientTransport, AgentServerTransport } from './transport'
import { AgentTransportError, FrameChannel } from './frame-channel'
import { clientFrameSchema } from './schemas'
import { serverFrameSchema } from './server-schemas'

export interface JsonWebSocket {
  send(data: string): void
  close(): void
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void
  addEventListener(type: 'error' | 'close', listener: () => void): void
  removeEventListener(type: 'message', listener: (event: { data: unknown }) => void): void
  removeEventListener(type: 'error' | 'close', listener: () => void): void
}

export interface FrameCodec<Send, Receive> {
  decode(value: unknown): Receive
  encode?(frame: Send): unknown
}

export function createWebSocketClientTransport(socket: JsonWebSocket): AgentClientTransport {
  return createWebSocketTransport<ClientFrame, ServerFrame>(socket, {
    decode: serverFrameSchema.parse,
  })
}

export function createWebSocketServerTransport(socket: JsonWebSocket): AgentServerTransport {
  return createWebSocketTransport<ServerFrame, ClientFrame>(socket, {
    decode: clientFrameSchema.parse,
  })
}

/** Custom protocols must supply their own validated inbound decoder. */
export function createWebSocketTransport<Send, Receive>(
  socket: JsonWebSocket,
  codec: FrameCodec<Send, Receive>,
): AgentTransport<Send, Receive> {
  return new WebSocketJsonTransport(socket, codec)
}

class WebSocketJsonTransport<Send, Receive> implements AgentTransport<Send, Receive> {
  private readonly channel = new FrameChannel<Receive>()
  private closed = false
  private readonly onMessage = (event: { data: unknown }): void => {
    if (this.closed) return
    let frame: Receive
    try {
      if (typeof event.data !== 'string') throw new Error('Agent frames must be text messages')
      frame = this.codec.decode(parsePortableJson(event.data))
    } catch {
      this.channel.report(new AgentTransportError('invalid_frame', 'Invalid agent frame'))
      return
    }
    void this.channel.receive(frame)
  }
  private readonly onClosed = (): void => {
    this.channel.report(new AgentTransportError('closed', 'Agent socket closed'))
    this.close()
  }
  private readonly onErrorEvent = (): void => {
    this.channel.report(new AgentTransportError('transport_error', 'Agent socket failed'))
    this.close()
  }

  constructor(
    private readonly socket: JsonWebSocket,
    private readonly codec: FrameCodec<Send, Receive>,
  ) {
    socket.addEventListener('message', this.onMessage)
    socket.addEventListener('close', this.onClosed)
    socket.addEventListener('error', this.onErrorEvent)
  }

  send(frame: Send): void {
    if (this.closed) throw new Error('Agent transport is closed')
    this.socket.send(stringifyPortableJson(this.codec.encode ? this.codec.encode(frame) : frame))
  }

  onFrame(handler: (frame: Receive) => void | Promise<void>): () => void {
    return this.channel.onFrame(handler)
  }

  onError(handler: (error: AgentTransportError) => void): () => void {
    return this.channel.onError(handler)
  }

  close(): void {
    if (this.closed) return
    this.closed = true
    this.channel.close()
    this.socket.removeEventListener('message', this.onMessage)
    this.socket.removeEventListener('close', this.onClosed)
    this.socket.removeEventListener('error', this.onErrorEvent)
    this.socket.close()
  }
}
