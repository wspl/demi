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
 * The transport is the client's as soon as the lane's open goes, so the
 * conversation's `open` follows the lane's at once, with no round trip
 * between them. A lane that is refused, cannot open or closes ends the
 * transport with a `ConversationLaneError`, the refusal's words when the
 * backend refused it, as the page connection's loss ends it too.
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
    return connectTransport(conversation)
  }, options)
}

/** Opens the lane of `conversation`, and answers its transport at once. */
function connectTransport(conversation: string): ConversationClientTransport {
  // An open that takes too long ends as a close (`openLane`).
  const lane = openLane({ kind: 'conversation', conversation }, 'conversation lane')
  const transport = createWebSocketTransport(lane)
  return {
    ...transport,
    onClose: (handler) => transport.onClose((error) => handler(new ConversationLaneError(lane.refusal?.error.message ?? error.message))),
  }
}
