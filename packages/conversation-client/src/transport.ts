import type { ClientFrame } from '@demicodes/protocol'

/**
 * A connection that carries frames; it does not vouch for them. What arrives
 * is handed over as a JSON value, and `AgentClient` validates it.
 */
export interface AgentClientTransport {
  send(frame: ClientFrame): void
  onFrame(handler: (frame: unknown) => void): () => void
  /** Called once when the connection ends without `close` being called. */
  onClose(handler: (error: Error) => void): () => void
  close(): void
}

/** What a socket's message carries. */
export interface SocketMessage {
  data: unknown
}

/** Why a socket closed. */
export interface SocketClose {
  code: number
  reason: string
}

/** The part of a browser `WebSocket` the transport uses. */
export interface WebSocketLike {
  send(data: string): void
  close(): void
  addEventListener(type: 'message', listener: (event: SocketMessage) => void): void
  addEventListener(type: 'close', listener: (event: SocketClose) => void): void
  addEventListener(type: 'error', listener: (event: unknown) => void): void
  removeEventListener(type: 'message', listener: (event: SocketMessage) => void): void
  removeEventListener(type: 'close', listener: (event: SocketClose) => void): void
  removeEventListener(type: 'error', listener: (event: unknown) => void): void
}

/**
 * The conversation socket as a transport: each frame is one JSON text
 * message. A message that is not JSON text is a peer that does not speak the
 * protocol, and the transport closes rather than skipping it.
 */
export function createWebSocketTransport(socket: WebSocketLike): AgentClientTransport {
  const frameHandlers = new Set<(frame: unknown) => void>()
  const closeHandlers = new Set<(error: Error) => void>()

  /** Stops listening and closes the socket; nothing is delivered after it. */
  const shut = () => {
    socket.removeEventListener('message', onMessage)
    socket.removeEventListener('close', onClose)
    socket.removeEventListener('error', onError)
    frameHandlers.clear()
    closeHandlers.clear()
    socket.close()
  }
  const end = (error: Error) => {
    const handlers = [...closeHandlers]
    shut()
    for (const handler of handlers) {
      handler(error)
    }
  }
  const onMessage = (event: SocketMessage) => {
    if (typeof event.data !== 'string') {
      end(new Error('The agent socket sent a binary message'))
      return
    }
    let frame: unknown
    try {
      frame = JSON.parse(event.data)
    } catch {
      end(new Error('The agent socket sent a message that is not JSON'))
      return
    }
    for (const handler of frameHandlers) {
      handler(frame)
    }
  }
  const onClose = (event: SocketClose) => {
    end(new Error(`The agent socket closed (${[event.code, event.reason].filter(Boolean).join(' ')})`))
  }
  const onError = () => {
    end(new Error('The agent socket failed'))
  }
  socket.addEventListener('message', onMessage)
  socket.addEventListener('close', onClose)
  socket.addEventListener('error', onError)

  return {
    send(frame) {
      socket.send(JSON.stringify(frame))
    },
    onFrame(handler) {
      frameHandlers.add(handler)
      return () => {
        frameHandlers.delete(handler)
      }
    },
    onClose(handler) {
      closeHandlers.add(handler)
      return () => {
        closeHandlers.delete(handler)
      }
    },
    close: shut,
  }
}
