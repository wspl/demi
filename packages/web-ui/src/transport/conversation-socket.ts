import { ConversationClient, createWebSocketTransport } from '@demicodes/conversation-client'
import { openSocket, watchSilence } from './liveness'

/**
 * The connection itself could not be made, was lost, or went silent: a
 * transport failure, not the session's, whether the session had answered
 * `open` yet or not. The runtime retries these on its own.
 */
export class ConversationSocketError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ConversationSocketError'
  }
}

/**
 * Opens the conversation socket at `url` and answers its client. A socket
 * lifetime is a connection lifetime, never a server-task lifetime. Until the
 * socket opens, this owns it and a failure is an `ConversationSocketError`; once it
 * opens, the client's transport owns it, and its close or failure disconnects
 * the client with an `ConversationSocketError` too. So does a socket that brings
 * nothing, heartbeats included, for as long as the liveness rule allows
 * (`web-application.md` § Liveness and reconnection).
 */
export function connectConversationClient(url: string, signal?: AbortSignal): Promise<ConversationClient> {
  return new Promise((resolve, reject) => {
    signal?.throwIfAborted()
    // A handshake that takes too long ends as a close (`openSocket`).
    const socket = openSocket(url)
    const release = () => {
      socket.removeEventListener('open', opened)
      socket.removeEventListener('error', failed)
      socket.removeEventListener('close', failed)
      signal?.removeEventListener('abort', aborted)
    }
    const failed = () => {
      release()
      socket.close()
      reject(new ConversationSocketError('Agent socket failed to connect'))
    }
    const aborted = () => {
      release()
      socket.close()
      reject(signal?.reason)
    }
    const opened = () => {
      release()
      const transport = createWebSocketTransport(socket)
      const client = new ConversationClient({
        ...transport,
        onClose: (handler) => transport.onClose((error) => handler(new ConversationSocketError(error.message))),
      })
      const silence = watchSilence(() => {
        client.disconnect(new ConversationSocketError('The agent socket went silent'))
      })
      const heard = () => silence.heard()
      socket.addEventListener('message', heard)
      // Every end of the client, the watch's own included, disconnects it.
      client.subscribe((event) => {
        if (event.type === 'disconnected') {
          silence.stop()
          socket.removeEventListener('message', heard)
        }
      })
      resolve(client)
    }
    socket.addEventListener('open', opened)
    socket.addEventListener('error', failed)
    socket.addEventListener('close', failed)
    signal?.addEventListener('abort', aborted, { once: true })
  })
}

/**
 * A connection made earlier, such as beside an opening's reads, for an
 * attempt that `signal` may end first: the attempt fails with the signal's
 * reason, and the connection, when it comes, is closed.
 */
export function takeConnection(made: Promise<ConversationClient>, signal: AbortSignal): Promise<ConversationClient> {
  return new Promise((resolve, reject) => {
    const aborted = () => reject(signal.reason)
    if (signal.aborted) {
      aborted()
    } else {
      signal.addEventListener('abort', aborted, { once: true })
    }
    made.then((client) => {
      signal.removeEventListener('abort', aborted)
      if (signal.aborted) {
        client.disconnect()
      } else {
        resolve(client)
      }
    }, (error: unknown) => {
      signal.removeEventListener('abort', aborted)
      reject(error)
    })
  })
}
