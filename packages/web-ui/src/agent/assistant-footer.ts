import type { Block, SessionPhase } from '@demicodes/protocol'
import type { TranscriptRequest } from '../files/request-changes'
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

/**
 * Where each request's Files Changed line goes (`edit-tracking.md` § What
 * the conversation shows): under the request's last row, by that row's id,
 * for a request whose calls changed a file, and only once that row ends the
 * reply (`replyEndIds`), as Copy and Fork wait for it. While the request
 * still works, in its turn or a later one it continues, it has no line.
 */
export function requestLineIds(
  rows: readonly Pick<Block, 'id'>[],
  requestOf: ReadonlyMap<string, TranscriptRequest>,
  ends: ReadonlySet<string>,
): Map<string, TranscriptRequest> {
  const last = new Map<TranscriptRequest, string>()
  for (const row of rows) {
    const request = requestOf.get(row.id)
    if (request && request.files.length > 0) {
      last.set(request, row.id)
    }
  }
  return new Map([...last].filter(([, id]) => ends.has(id)).map(([request, id]) => [id, request]))
}
