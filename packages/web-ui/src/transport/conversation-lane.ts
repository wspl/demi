import {
  ConversationClient,
  createWebSocketTransport,
  type ConversationClientOptions,
  type ConversationClientTransport,
} from '@demicodes/conversation-client'
import { openLane } from './page-connection'

/**
 * The lane itself could not be opened, was refused, or closed: a transport
 * failure, not the session's, whether the session had answered `open` yet or
 * not. The runtime retries these on its own.
 */
export class ConversationLaneError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ConversationLaneError'
  }
}

/**
 * The client of the conversation `conversation`, over its lane of the page
 * connection. The client reads first, then calls this to open the lane, once
 * per connection; a lane's lifetime is a connection's lifetime, never a
 * server task's.
 *
 * - Until the lane opens, this owns it and a failure is a
 *   `ConversationLaneError`.
 * - Once it opens, the client's transport owns it, and its close disconnects
 *   the client with a `ConversationLaneError` too, as the page connection's
 *   loss closes it.
 *
 * `options` are the client's, such as the conversation's own frames it takes.
 * `before` answers what each connection waits for, such as a restore the
 * backend must finish before it accepts the lane.
 */
export function createConversationClient(
  conversation: string,
  options: ConversationClientOptions = {},
  before: () => Promise<unknown> | undefined = () => undefined,
): ConversationClient {
  return new ConversationClient(async (signal) => {
    await before()
    signal.throwIfAborted()
    return connectTransport(conversation, signal)
  }, options)
}

/** Opens the lane of `conversation`, and answers its transport. */
function connectTransport(conversation: string, signal: AbortSignal): Promise<ConversationClientTransport> {
  return new Promise((resolve, reject) => {
    signal.throwIfAborted()
    // An open that takes too long ends as a close (`openLane`).
    const lane = openLane({ kind: 'conversation', conversation }, 'conversation lane')
    const release = () => {
      lane.removeEventListener('open', opened)
      lane.removeEventListener('close', failed)
      signal.removeEventListener('abort', aborted)
    }
    const failed = () => {
      release()
      lane.close()
      const refused = lane.refusal?.error.message
      reject(new ConversationLaneError(refused ?? 'The conversation lane could not be opened'))
    }
    const aborted = () => {
      release()
      lane.close()
      reject(signal.reason)
    }
    const opened = () => {
      release()
      const transport = createWebSocketTransport(lane)
      resolve({
        ...transport,
        onClose: (handler) => transport.onClose((error) => handler(new ConversationLaneError(error.message))),
      })
    }
    lane.addEventListener('open', opened)
    lane.addEventListener('close', failed)
    signal.addEventListener('abort', aborted, { once: true })
  })
}
