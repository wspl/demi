import {
  ConversationClient,
  createWebSocketTransport,
  type ConversationClientOptions,
  type ConversationClientTransport,
} from '@demicodes/conversation-client'
import { openSocket, pingOn, watchSilence } from './liveness'

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
 * The client of the conversation whose socket is at `url`. The client reads
 * first, then calls this to open the socket, once per connection; a socket
 * lifetime is a connection lifetime, never a server-task lifetime.
 *
 * - Until the socket opens, this owns it and a failure is a
 *   `ConversationSocketError`.
 * - Once it opens, the client's transport owns it, and its close or failure
 *   disconnects the client with a `ConversationSocketError` too. So does a socket
 *   that brings nothing, heartbeats included, for as long as the liveness rule
 *   allows.
 *
 * `options` are the client's, such as the conversation's own frames it takes.
 * `before` answers what each connection waits for, such as a restore the
 * backend must finish before it accepts the socket.
 */
export function createConversationClient(
  url: string,
  options: ConversationClientOptions = {},
  before: () => Promise<unknown> | undefined = () => undefined,
): ConversationClient {
  const client: ConversationClient = new ConversationClient(async (signal) => {
    await before()
    signal.throwIfAborted()
    return connectTransport(url, signal, client)
  }, options)
  return client
}

/** Opens the socket at `url` for `client`, and answers its transport, which `client` is to attach. */
function connectTransport(url: string, signal: AbortSignal, client: ConversationClient): Promise<ConversationClientTransport> {
  return new Promise((resolve, reject) => {
    signal.throwIfAborted()
    // A handshake that takes too long ends as a close (`openSocket`).
    const socket = openSocket(url, 'conversation socket')
    const release = () => {
      socket.removeEventListener('open', opened)
      socket.removeEventListener('error', failed)
      socket.removeEventListener('close', failed)
      signal.removeEventListener('abort', aborted)
    }
    const failed = () => {
      release()
      socket.close()
      reject(new ConversationSocketError('Agent socket failed to connect'))
    }
    const aborted = () => {
      release()
      socket.close()
      reject(signal.reason)
    }
    const opened = () => {
      release()
      const transport = createWebSocketTransport(socket)
      const silence = watchSilence('conversation socket', () => {
        client.disconnect(new ConversationSocketError('The agent socket went silent'))
      }, pingOn(socket))
      const heard = () => silence.heard()
      socket.addEventListener('message', heard)
      const stop = () => {
        silence.stop()
        socket.removeEventListener('message', heard)
        unsubscribe()
      }
      // Every end of the connection, the watch's own included, disconnects
      // the client, which then stops the watch.
      const unsubscribe = client.subscribe((event) => {
        if (event.type === 'pong') {
          silence.ponged(event.id)
        } else if (event.type === 'disconnected') {
          stop()
        }
      })
      resolve({
        ...transport,
        onClose: (handler) => transport.onClose((error) => handler(new ConversationSocketError(error.message))),
        close: () => {
          stop()
          transport.close()
        },
      })
    }
    socket.addEventListener('open', opened)
    socket.addEventListener('error', failed)
    socket.addEventListener('close', failed)
    signal.addEventListener('abort', aborted, { once: true })
  })
}
