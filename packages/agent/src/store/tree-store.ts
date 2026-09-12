import type { AgentSessionPersistUpdate } from '../types'

const COMPLETION_PREFIX = 'subagent:'

/**
 * The id of the agent receipt that wakes or steers a parent with a child's completion:
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
 * The children whose completion receipt a checkpoint carries as pending
 * internal input or a materialized agent_message block (`docs/subagent.md` § Persistence).
 */
export function completedChildrenCarriedBy(
  update: Pick<AgentSessionPersistUpdate<unknown>, 'changedBlocks' | 'pendingInternalSteers'>
): Array<{ id: string; spawnedAt: number }> {
  const rounds = new Map<string, { id: string; spawnedAt: number }>()
  const messages = [
    ...(update.pendingInternalSteers ?? []).map((steer) => steer.agentMessage),
    ...update.changedBlocks.flatMap(({ block }) => block.type === 'agent_message' ? [block.message] : []),
  ]
  for (const message of messages) {
    if (message.event.type !== 'completion') {
      continue
    }
    const round = childRoundOf(message.id)
    if (!round || round.id !== message.sender.id || round.spawnedAt !== message.sender.round) {
      throw new Error('Completion identity does not match its source round')
    }
    rounds.set(message.id, round)
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
