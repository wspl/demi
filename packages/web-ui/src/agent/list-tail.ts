import type { Block, PendingCall, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { pendingSteersToRenderBlocks, type MessageListBlock, type PendingCallRenderBlock } from './pending-steers'
import { queuedMessagesToRenderBlocks } from './queued-messages'
import type { PendingSteerMessage, PendingSubmissionState } from './types'

/**
 * A compaction in progress: its divider, at the end of the transcript, where
 * the pass appends the marker that shows the finished one
 * (`runtime.md` § Block types).
 */
export interface CompactionProgressBlock {
  type: 'compaction_progress'
  id: 'compaction-progress'
}

export interface ListTailInput {
  phase: SessionPhase
  /** The transcript the tail follows. */
  blocks: readonly Block[]
  pendingCalls: readonly PendingCall[]
  pendingSteers: readonly PendingSteerMessage[]
  queue: readonly QueuedMessage[]
  pendingSubmission?: PendingSubmissionState | null
}

/** What the list shows after the transcript, in two parts the tail row stands between. */
export interface ListTail {
  /**
   * The turn's own rows: a compaction in progress, the calls the model is
   * writing, and a message sent as a message, which is the user's next
   * message, so the tail row's Requesting stands under it.
   */
  turn: MessageListBlock[]
  /**
   * The messages that wait below the turn (`product.md` § Following the
   * agent's work): the steers, then the queue under its divider, each with
   * the message on its way that will be one of them, shown from its send as
   * what it will be.
   */
  waiting: MessageListBlock[]
}

/**
 * What the list shows after the transcript. Steers that arrive during a
 * compaction wait outside what it summarizes, so they follow its divider. A
 * message the session holds already shows there alone, in the transcript, as
 * a steer or in the queue: the row it was waiting in becomes that one
 * (`row-key.ts`), so it never shows twice, whichever of the two the server
 * tells first.
 */
export function listTailBlocks(input: ListTailInput): ListTail {
  const written = new Set(input.blocks.flatMap((block) =>
    block.type === 'user' ? [block.turnId] : block.type === 'steer' ? [block.id] : []))
  const steers = input.pendingSteers.filter((steer) => !written.has(steer.id))
  const queue = input.queue.filter((message) => !written.has(message.id))
  const submission = input.pendingSubmission
  const held = !submission || written.has(submission.id)
    || steers.some((steer) => steer.id === submission.id)
    || queue.some((message) => message.id === submission.id)
  const sending: MessageListBlock[] = held ? [] : [{
    type: 'pending_submission',
    id: `pending-submission:${submission.id}`,
    submission,
  }]
  const way = held ? null : submission.way
  const queued = [...queuedMessagesToRenderBlocks(queue), ...(way === 'queue' ? sending : [])]
  return {
    turn: [
      ...(input.phase === 'compacting'
        ? [{ type: 'compaction_progress', id: 'compaction-progress' } as const]
        : []),
      ...pendingCallsToRenderBlocks(input.pendingCalls, input.blocks),
      ...(way === 'send' ? sending : []),
    ],
    waiting: [
      ...pendingSteersToRenderBlocks(steers),
      ...(way === 'steer' ? sending : []),
      ...(queued.length > 0
        ? [{ type: 'queue_divider', id: 'queue-divider', count: queued.length } as const, ...queued]
        : []),
    ],
  }
}

/**
 * The calls the model is writing, each as its row, without a call whose
 * block the transcript holds already: the block takes its place, so the
 * call never shows twice (`runtime.md` § Calls being written).
 */
export function pendingCallsToRenderBlocks(
  calls: readonly PendingCall[],
  blocks: readonly Block[],
): PendingCallRenderBlock[] {
  if (calls.length === 0) {
    return []
  }
  const held = new Set(blocks.flatMap((block) => (block.type === 'tool_call' ? [block.toolUseId] : [])))
  return calls
    .filter((call) => !held.has(call.toolUseId))
    .map((call) => ({ type: 'pending_call', id: `pending-call:${call.toolUseId}`, call }))
}
