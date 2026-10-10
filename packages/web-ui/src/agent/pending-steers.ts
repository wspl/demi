import type { WorkGroupBlock } from './work-groups'
import type { Block, PendingCall, UserContentBlock } from '@demicodes/protocol'
import type { CompactionProgressBlock } from './list-tail'
import type { QueueDividerBlock, QueuedRenderBlock } from './queued-messages'
import type { PendingSteerMessage, PendingSubmissionState } from './types'

export interface PendingSteerRenderBlock {
  type: 'pending_steer'
  id: string
  pendingSteerId: string
  content: UserContentBlock[]
}

/** A call the model is writing, shown after the transcript (`runtime.md` § Calls being written). */
export interface PendingCallRenderBlock {
  type: 'pending_call'
  id: string
  call: PendingCall
}

export type MessageListBlock =
  Block | PendingSteerRenderBlock | QueueDividerBlock | QueuedRenderBlock | CompactionProgressBlock |
  PendingCallRenderBlock | WorkGroupBlock |
  { type: 'pending_submission'; id: string; submission: PendingSubmissionState } |
  /** The edge of a window that does not reach its transcript's start or end, which shows its next page coming. */
  { type: 'history_edge'; id: 'history-before' | 'history-after' }

export function pendingSteersToRenderBlocks(
  pendingSteers: readonly PendingSteerMessage[]
): PendingSteerRenderBlock[] {
  return pendingSteers.map((pending) => ({
    type: 'pending_steer',
    id: `pending-steer:${pending.id}`,
    pendingSteerId: pending.id,
    content: pending.content,
  }))
}
