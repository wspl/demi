import type { Block, SessionPhase } from '@demicodes/protocol'
import { isCompactionDivider } from './visible-blocks'

/**
 * The blocks that end their reply: after each, the work it belongs to is
 * done. Visible transcript order tells them from intermediate updates: the
 * next block is the user's next message or the user's Stop, or there is
 * none and the session is idle. Anything else that follows (a call, a
 * steer, a wakeup, another agent's message, a resume) continues the work.
 * A compaction's divider after a block leaves it the end: the user
 * compacted after it, and nothing of the turn followed.
 */
export function replyEndIds(
  visibleBlocks: readonly Pick<Block, 'id' | 'type'>[],
  phase: SessionPhase,
): Set<string> {
  const turnBlocks = visibleBlocks.filter((block) => !isCompactionDivider(block))
  const ids = new Set<string>()
  turnBlocks.forEach((block, index) => {
    const next = turnBlocks[index + 1]
    if (next?.type === 'user' || next?.type === 'abort' || (!next && phase === 'idle')) {
      ids.add(block.id)
    }
  })
  return ids
}

/**
 * The replies that carry Copy and Fork: the text blocks among the reply
 * ends `ends` (`replyEndIds`). The user's Stop leaves the reply it cut off
 * the turn's last word, and it keeps its Copy.
 */
export function assistantFooterIds(
  visibleBlocks: readonly Pick<Block, 'id' | 'type'>[],
  ends: ReadonlySet<string>,
): Set<string> {
  return new Set(visibleBlocks.filter((block) => block.type === 'text' && ends.has(block.id)).map((block) => block.id))
}
