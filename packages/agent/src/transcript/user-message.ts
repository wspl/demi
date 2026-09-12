import type { Block } from '@demicodes/core'

/** User submissions have edit semantics; internal wakeups and steers do not. */
export function isEditableUserMessage(
  block: Block,
): block is Extract<Block, { type: 'user' }> {
  return block.type === 'user' && !block.hidden
}
