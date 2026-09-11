import { ref, watch } from 'vue'
import { reportError } from '../infra/errors'

export interface MessageForkRequest {
  id: string
  blockId: string
}
export type MessageForkHandler = (request: MessageForkRequest) => Promise<void>
/** A fork in flight. A failed fork is a toast and leaves no state behind; the same request id retries it. */
export interface MessageForkState {
  phase: 'pending'
  request: MessageForkRequest
}

/** Keep request IDs above virtualized rows, with one independent action per message. */
export function useMessageForks(
  handler: () => MessageForkHandler | undefined,
  conversationId: () => string,
) {
  const states = ref(new Map<string, MessageForkState>())
  // The id of a failed request, so a retry names the same operation to the server.
  const failedRequests = new Map<string, MessageForkRequest>()
  watch(conversationId, () => {
    states.value = new Map()
    failedRequests.clear()
  }, { flush: 'sync' })

  async function run(blockId: string): Promise<void> {
    const execute = handler()
    const scope = states.value
    if (!execute || scope.has(blockId)) {
      return
    }
    const request = failedRequests.get(blockId) ?? { id: crypto.randomUUID(), blockId }
    scope.set(blockId, { phase: 'pending', request })
    try {
      await execute(request)
      failedRequests.delete(blockId)
    } catch (error) {
      // A late outcome from a conversation that was switched away has no message to retry.
      if (states.value === scope) {
        failedRequests.set(blockId, request)
      }
      reportError('Could not fork from this message', error, { userVisible: true })
    } finally {
      scope.delete(blockId)
    }
  }

  return { states, run }
}
