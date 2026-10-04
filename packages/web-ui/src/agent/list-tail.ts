import type { QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { pendingSteersToRenderBlocks, type MessageListBlock } from './pending-steers'
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
  pendingSteers: readonly PendingSteerMessage[]
  queue: readonly QueuedMessage[]
  pendingSubmission?: PendingSubmissionState | null
}

/**
 * What the list shows after the transcript, in order: a compaction in
 * progress, the steers waiting for the turn, the queue, and a message sent
 * but not yet confirmed. Steers that arrive during a pass wait outside what
 * it summarizes, so they follow its divider.
 */
export function listTailBlocks(input: ListTailInput): MessageListBlock[] {
  return [
    ...(input.phase === 'compacting'
      ? [{ type: 'compaction_progress', id: 'compaction-progress' } as const]
      : []),
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
