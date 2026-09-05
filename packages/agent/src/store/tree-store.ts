import type { AgentSessionPersistUpdate } from '../types'

const COMPLETION_PREFIX = 'subagent:'

/**
 * The id of the user message that wakes a parent with a child's completion:
 * one per child, so a checkpoint names the completions it carries and a
 * store can mark them delivered in the same commit as the save.
 */
export function completionMessageId(childId: string): string {
  return `${COMPLETION_PREFIX}${childId}`
}

/**
 * The children whose completion message a checkpoint carries — still queued,
 * or as the user turn it opened (`docs/subagent.md` § Persistence).
 */
export function completedChildrenCarriedBy(update: Pick<AgentSessionPersistUpdate<unknown>, 'queue' | 'changedBlocks'>): string[] {
  const ids = new Set<string>()
  for (const message of update.queue) {
    const id = childOf(message.id)
    if (id) ids.add(id)
  }
  for (const { block } of update.changedBlocks) {
    if (block.type !== 'user') continue
    const id = childOf(block.turnId)
    if (id) ids.add(id)
  }
  return [...ids]
}

function childOf(messageId: string): string | null {
  return messageId.startsWith(COMPLETION_PREFIX) ? messageId.slice(COMPLETION_PREFIX.length) : null
}
