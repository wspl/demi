import type { ConversationMetadata } from '../api/generated/web-api'
import { useConversations } from '../conversation/store'
import { conversationMetadata } from './product-state'

// Apart from `./product-state.ts`, which tests on a happy-dom page import
// before the page registers: the store loads Vue's DOM runtime, which takes
// the document it finds then.
/**
 * Shows the conversation `id` with the metadata `parts` make, as its
 * socket's `conversation` frame does when the page shows it.
 */
export function showMetadata(id: string, parts: Partial<ConversationMetadata> = {}): void {
  const conversation = useConversations().items.find((item) => item.id === id)
  if (!conversation) {
    throw new Error(`no conversation ${id}`)
  }
  conversation.metadata.replace(conversationMetadata(id, parts))
}
