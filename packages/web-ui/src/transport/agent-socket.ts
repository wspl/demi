import { AgentClient, createWebSocketTransport } from '@demicodes/agent-client'

/**
 * The connection itself could not be made or was lost before it opened: a
 * transport failure, not the session's. The runtime retries these on its own.
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
 * the client.
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
      resolve(new AgentClient(createWebSocketTransport(socket)))
    }
    const timeout = setTimeout(failed, CONNECT_TIMEOUT_MS)
    socket.addEventListener('open', opened)
    socket.addEventListener('error', failed)
    socket.addEventListener('close', failed)
    signal?.addEventListener('abort', aborted, { once: true })
  })
}
