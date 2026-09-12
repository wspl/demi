import type { AgentSessionPersistUpdate } from '../types'

const COMPLETION_PREFIX = 'subagent:'

/**
 * The id of the user message that wakes a parent with a child's completion:
 * one per execution round, so a checkpoint names the completions it carries and a
 * store can mark them delivered in the same commit as the save.
 */
export function completionMessageId(childId: string, spawnedAt: number): string {
  return `${COMPLETION_PREFIX}${childId}:${spawnedAt}`
}

export function isCompletionMessageId(messageId: string): boolean {
  return messageId.startsWith(COMPLETION_PREFIX)
}

/**
 * The children whose completion message a checkpoint carries — still queued,
 * or as the user turn it opened (`docs/subagent.md` § Persistence).
 */
export function completedChildrenCarriedBy(
  update: Pick<AgentSessionPersistUpdate<unknown>, 'queue' | 'changedBlocks'>
): Array<{ id: string; spawnedAt: number }> {
  const rounds = new Map<string, { id: string; spawnedAt: number }>()
  for (const message of update.queue) {
    const round = childRoundOf(message.id)
    if (round)
      rounds.set(message.id, round)
  }
  for (const { block } of update.changedBlocks) {
    if (block.type !== 'user')
      continue
    const round = childRoundOf(block.turnId)
    if (round)
      rounds.set(block.turnId, round)
  }
  return [...rounds.values()]
}

function childRoundOf(messageId: string): { id: string; spawnedAt: number } | null {
  if (!isCompletionMessageId(messageId)) {
    return null
  }
  const separator = messageId.lastIndexOf(':')
  const id = messageId.slice(COMPLETION_PREFIX.length, separator)
  const time = messageId.slice(separator + 1)
  if (!id || !/^\d+$/.test(time) || !Number.isSafeInteger(Number(time))) {
    throw new Error('Invalid subagent completion message id')
  }
  return { id, spawnedAt: Number(time) }
}
