import { ref, watch } from 'vue'

export interface MessageForkRequest {
  id: string
  blockId: string
}
export type MessageForkHandler = (request: MessageForkRequest) => Promise<void>
export type MessageForkState =
  | { phase: 'pending'; request: MessageForkRequest }
  | { phase: 'failed'; request: MessageForkRequest; error: string }

/** Keep retry IDs above virtualized rows, with one independent action per message. */
export function useMessageForks(
  handler: () => MessageForkHandler | undefined,
  conversationId: () => string,
) {
  const states = ref(new Map<string, MessageForkState>())
  watch(conversationId, () => { states.value = new Map() }, { flush: 'sync' })

  async function run(blockId: string): Promise<void> {
    const execute = handler()
    const scope = states.value
    const existing = scope.get(blockId)
    if (!execute || existing?.phase === 'pending') {
      return
    }
    const request = existing?.request ?? { id: crypto.randomUUID(), blockId }
    scope.set(blockId, { phase: 'pending', request })
    try {
      await execute(request)
      scope.delete(blockId)
    } catch (error) {
      scope.set(blockId, {
        phase: 'failed', request,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  return { states, run }
}
