import type { WorkGroupBlock } from './work-groups'
import type { HeldBlock } from '@demicodes/conversation-client'
import type { MessageComment, PendingCall, UserContentBlock } from '@demicodes/protocol'
import type { CompactionProgressBlock } from './list-tail'
import type { QueueDividerBlock, QueuedRenderBlock } from './queued-messages'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import type { ActivitySlotState } from './useActivitySlot'
import type { SessionFailureNotice } from './session-status'

export interface PendingSteerRenderBlock {
  type: 'pending_steer'
  id: string
  pendingSteerId: string
  content: UserContentBlock[]
  /** The comments it carries; absent with none. */
  comments?: MessageComment[]
}

/**
 * A call the model is writing whose description it has written, shown after
 * the transcript, titled by it.
 */
export interface PendingCallRenderBlock {
  type: 'pending_call'
  id: string
  call: PendingCall & { description: string }
}

/**
 * The transcript's tail row, Requesting or Connecting, right after the
 * transcript and above the messages that wait. Each wait it shows is a row of
 * its own, which enters once.
 */
export interface ActivitySlotRow {
  type: 'activity_slot'
  id: `tail-row:${number}`
  slot: ActivitySlotState
}

/**
 * A message whose delivery failed, told under it with Retry: for a message
 * sent to start a turn, in the place of the Requesting row it replaces, which
 * Retry rolls it back into.
 */
export interface DeliveryFailureRow {
  type: 'delivery_failure'
  id: `delivery-failure:${string}`
  submission: PendingSubmissionState
}

/** A session-level failure, told at the tail of the transcript, in the tail row's place. */
export interface SessionFailureRow {
  type: 'session_failure'
  id: 'session-failure'
  failure: SessionFailureNotice
}

/**
 * A run of the agent's steps a share does not hold, in its place after the
 * row of the block `after`, or before the first row for null: one muted line
 * that does not open.
 */
export interface StepsNotSharedRow {
  type: 'steps_not_shared'
  id: string
  count: number
  after: string | null
}

export type MessageListBlock =
  HeldBlock | PendingSteerRenderBlock | QueueDividerBlock | QueuedRenderBlock | CompactionProgressBlock |
  PendingCallRenderBlock | WorkGroupBlock | ActivitySlotRow | DeliveryFailureRow | SessionFailureRow | StepsNotSharedRow |
  /** A message on its way; `startsTurn` when it is a steer that starts the turn, which reads as the user's next message. */
  { type: 'pending_submission'; id: string; submission: PendingSubmissionState; startsTurn: boolean } |
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
    comments: pending.comments,
  }))
}

/**
 * What a message row says waits for the agent, with its line and its Send
 * Now and Recall: a steer or a queued message, and a message on its way that
 * will be one of them, from its send, unless its delivery failed or it is a
 * steer that starts the turn, which is the user's next message; undefined for
 * any other row.
 */
export function waitingAs(row: MessageListBlock): 'steer' | 'queued' | undefined {
  switch (row.type) {
    case 'pending_steer':
      return 'steer'
    case 'queued_message':
      return 'queued'
    case 'pending_submission':
      if (row.submission.error !== null || row.startsTurn) {
        return undefined
      }
      return row.submission.way === 'steer' ? 'steer' : row.submission.way === 'queue' ? 'queued' : undefined
    default:
      return undefined
  }
}
