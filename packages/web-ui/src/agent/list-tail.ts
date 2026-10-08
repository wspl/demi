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

/**
 * What the list shows after the transcript, in order: a compaction in
 * progress, the calls the model is writing, the steers waiting for the turn,
 * the queue, and a message sent but not yet confirmed. Steers that arrive
 * during a pass wait outside what it summarizes, so they follow its divider.
 */
export function listTailBlocks(input: ListTailInput): MessageListBlock[] {
  return [
    ...(input.phase === 'compacting'
      ? [{ type: 'compaction_progress', id: 'compaction-progress' } as const]
      : []),
    ...pendingCallsToRenderBlocks(input.pendingCalls, input.blocks),
    ...pendingSteersToRenderBlocks(input.pendingSteers),
    ...queuedMessagesToRenderBlocks(input.queue),
    ...(input.pendingSubmission
      ? [{
          type: 'pending_submission' as const,
          id: `pending-submission:${input.pendingSubmission.id}`,
          submission: input.pendingSubmission,
        }]
      : []),
  ]
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
