import { AgentClient, createWebSocketTransport } from '@demicodes/agent-client'
import { watchSilence } from './liveness'

/**
 * The connection itself could not be made, was lost, or went silent: a
 * transport failure, not the session's, whether the session had answered
 * `open` yet or not. The runtime retries these on its own.
 */
export class AgentSocketError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'AgentSocketError'
  }
}

/** How long a socket may take to open. */
const CONNECT_TIMEOUT_MS = 15_000

/**
 * Opens the conversation socket at `url` and answers its client. A socket
 * lifetime is a connection lifetime, never a server-task lifetime. Until the
 * socket opens, this owns it and a failure is an `AgentSocketError`; once it
 * opens, the client's transport owns it, and its close or failure disconnects
 * the client with an `AgentSocketError` too. So does a socket that brings
 * nothing, heartbeats included, for as long as the liveness rule allows
 * (`web-application.md` § Liveness and reconnection).
 */
export function connectAgentClient(url: string, signal?: AbortSignal): Promise<AgentClient> {
  return new Promise((resolve, reject) => {
    signal?.throwIfAborted()
    const socket = new WebSocket(url)
    const release = () => {
      clearTimeout(timeout)
      socket.removeEventListener('open', opened)
      socket.removeEventListener('error', failed)
      socket.removeEventListener('close', failed)
      signal?.removeEventListener('abort', aborted)
    }
    const failed = () => {
      release()
      socket.close()
      reject(new AgentSocketError('Agent socket failed to connect'))
    }
    const aborted = () => {
      release()
      socket.close()
      reject(signal?.reason)
    }
    const opened = () => {
      release()
      const transport = createWebSocketTransport(socket)
      const client = new AgentClient({
        ...transport,
        onClose: (handler) => transport.onClose((error) => handler(new AgentSocketError(error.message))),
      })
      const silence = watchSilence(() => {
        client.disconnect(new AgentSocketError('The agent socket went silent'))
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
    const timeout = setTimeout(failed, CONNECT_TIMEOUT_MS)
    socket.addEventListener('open', opened)
    socket.addEventListener('error', failed)
    socket.addEventListener('close', failed)
    signal?.addEventListener('abort', aborted, { once: true })
  })
}
