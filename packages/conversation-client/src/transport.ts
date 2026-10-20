import { MAX_PAGE_MESSAGE_BYTES, type ClientFrame } from '@demicodes/protocol'

/**
 * A connection that carries frames; it does not vouch for them. What arrives
 * is handed over as a JSON value, and `ConversationClient` validates it.
 */
export interface ConversationClientTransport {
  /** Sends `frame`; a frame over the backend's limit throws `FrameTooLargeError` and sends nothing. */
  send(frame: ClientFrame): void
  onFrame(handler: (frame: unknown) => void): () => void
  /** Called once when the connection ends without `close` being called. */
  onClose(handler: (error: Error) => void): () => void
  close(): void
}

/** A frame larger than the backend takes on a page's socket. */
export class FrameTooLargeError extends Error {
  constructor(bytes: number) {
    super(`The message is ${bytes} bytes, over the ${MAX_PAGE_MESSAGE_BYTES} bytes the conversation socket takes`)
    this.name = 'FrameTooLargeError'
  }
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

/** The part of the web browser's `WebSocket` the transport uses. */
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
 * The conversation socket as a transport: each message is JSON text, one
 * frame or an array of the frames the backend had ready at once, as a
 * turn's end, which are handed over in order within the message's event, so
 * the page applies them as one change. A message that is not JSON text is a
 * peer that does not speak the protocol, and the transport closes rather than
 * skipping it.
 */
export function createWebSocketTransport(socket: WebSocketLike): ConversationClientTransport {
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
    let message: unknown
    try {
      message = JSON.parse(event.data)
    } catch {
      end(new Error('The agent socket sent a message that is not JSON'))
      return
    }
    // A handler that ends the connection clears the handlers, so no frame
    // after it is handed over.
    for (const frame of Array.isArray(message) ? message : [message]) {
      for (const handler of frameHandlers) {
        handler(frame)
      }
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
      const text = JSON.stringify(frame)
      // A UTF-16 unit is at most three UTF-8 bytes, so a shorter text needs no count.
      if (text.length * 3 > MAX_PAGE_MESSAGE_BYTES) {
        const bytes = new TextEncoder().encode(text).byteLength
        if (bytes > MAX_PAGE_MESSAGE_BYTES) {
          throw new FrameTooLargeError(bytes)
        }
      }
      socket.send(text)
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
