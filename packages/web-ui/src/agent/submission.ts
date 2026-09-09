import type { ConversationState } from './types'

/** Admission is visible in either the transcript or the server queue. */
export function hasAcceptedSubmission(
  state: Pick<ConversationState, 'blocks' | 'queue'>,
  id: string,
): boolean {
  return state.blocks.some((block) => block.type === 'user' && block.turnId === id) ||
    state.queue.some((message) => message.id === id)
}
