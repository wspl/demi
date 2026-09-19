import type { Block, UserContentBlock } from '@demicodes/core'
import type { QueueDividerBlock, QueuedRenderBlock } from './queued-messages'
import type { PendingSteerMessage, PendingSubmissionState } from './types'

export interface PendingSteerRenderBlock {
  type: 'pending_steer'
  id: string
  pendingSteerId: string
  content: UserContentBlock[]
}

export type MessageListBlock =
  Block | PendingSteerRenderBlock | QueueDividerBlock | QueuedRenderBlock |
  { type: 'pending_submission'; id: string; submission: PendingSubmissionState }

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
