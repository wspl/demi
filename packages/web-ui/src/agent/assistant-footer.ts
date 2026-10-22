import type { Block, SessionPhase } from '@demicodes/protocol'
import { isCompactionDivider } from './visible-blocks'

/** What `replyEndIds` reads of a block. */
type ReplyBlock = Pick<Block, 'id' | 'type'> & { forkable?: boolean }

/** What `replyEndIds` reads besides the blocks. */
export interface ReplyEndInput {
  phase: SessionPhase
  /**
   * The agent goes on within its turn: the server says it waits for the model's
   * answer, or a call is being written. Text the model finished is the end of
   * its reply only when nothing goes on after it.
   */
  continuing: boolean
  /**
   * A message being sent is the user's next message from the moment it is sent,
   * so the last block stays the end while the message is delivered and its
   * turn starts.
   */
  sending?: boolean
}

/**
 * The blocks that end their reply: after each, the work it belongs to is done.
 *
 * Visible transcript order tells them from intermediate updates. After a block
 * that ends its reply, the next block is the user's next message or the user's
 * Stop, or there is none and nothing goes on. Anything else that follows (a
 * call, a steer, a wakeup, another agent's message, a resume) continues the
 * work.
 *
 * With none after it, a block is the end when the session is idle or the next
 * message is on its way, and a text the backend marked complete (`forkable`)
 * is the end from that moment, without waiting for the session to be idle,
 * unless the agent goes on.
 *
 * A compaction's divider after a block leaves it the end: the user compacted
 * after it, and nothing of the turn followed.
 */
export function replyEndIds(
  visibleBlocks: readonly ReplyBlock[],
  input: ReplyEndInput,
): Set<string> {
  const turnBlocks = visibleBlocks.filter((block) => !isCompactionDivider(block))
  const ids = new Set<string>()
  turnBlocks.forEach((block, index) => {
    const next = turnBlocks[index + 1]
    const last = !next && (
      input.phase === 'idle' || input.sending === true || (block.type === 'text' && block.forkable === true && !input.continuing)
    )
    if (next?.type === 'user' || next?.type === 'abort' || last) {
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
