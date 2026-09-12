import type { Block } from '@demicodes/core'
import { isCompletionMessageId } from '../store/tree-store'

/** User submissions have edit semantics; internal wakeups and steers do not. */
export function isEditableUserMessage<B extends Block<unknown, unknown>>(
  block: B,
): block is Extract<B, { type: 'user' }> {
  return block.type === 'user' && !block.hidden
    && !isCompletionMessageId(block.turnId)
}
