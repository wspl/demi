import type { WorkGroupBlock } from './work-groups'
import type { HeldBlock } from '@demicodes/conversation-client'
import type { MessageComment, PendingCall, QueuedMessage, UserContentBlock } from '@demicodes/protocol'
import type { CompactionProgressBlock } from './list-tail'
import type { QueueDividerBlock, QueuedRenderBlock } from './queued-messages'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import type { ActivitySlotState } from './useActivitySlot'
import type { SessionFailureNotice } from './session-status'
import { operations, type Operation } from '../ui/operations'
import { inject, provide, type InjectionKey } from 'vue'

/**
 * Send on the waiting message `id`, a steer or a queued message the session
 * holds: its button shows the spinner while it runs, from the press until the
 * session takes it, which a slow way to the backend can make seconds.
 */
export function sendNowOperation(id: string): Operation {
  return { action: 'send-now', target: `waiting:${id}` }
}

/**
 * What Send on a waiting message does: the steer `id` interrupts the agent's
 * step and goes to the model, the queued message `id` goes without waiting
 * for the turn to end.
 */
export interface WaitingSend {
  as: 'steer' | 'queued'
  id: string
}

/**
 * The waiting message Return in an empty composer sends: the first of
 * `waiting` whose Send is not on its way already, so each press sends the
 * next one.
 */
export function nextWaitingSend(waiting: readonly WaitingSend[]): WaitingSend | undefined {
  return waiting.find((send) => !operations.blocked(sendNowOperation(send.id)))
}

const sendNextWaitingKey: InjectionKey<() => (() => void) | undefined> = Symbol('send-next-waiting')

/**
 * What the main composer's Return does in an empty composer, read at each
 * press and as the placeholder shows: sends the next waiting message now, as
 * its Send does, and is absent while none waits.
 */
export function provideSendNextWaiting(handler: () => (() => void) | undefined): void {
  provide(sendNextWaitingKey, handler)
}

export function useSendNextWaiting(): () => (() => void) | undefined {
  return inject(sendNextWaitingKey, () => undefined)
}

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
 * The transcript's tail row, Requesting, right after the
 * transcript and above the messages that wait. Each wait it shows is a row of
 * its own, which enters once.
 */
export interface ActivitySlotRow {
  type: 'activity_slot'
  id: `tail-row:${number}`
  slot: ActivitySlotState
}

/**
 * A session-level failure, told at the tail of the transcript, in the tail
 * row's place.
 */
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
  PendingCallRenderBlock | WorkGroupBlock | ActivitySlotRow | SessionFailureRow | StepsNotSharedRow |
  /**
   * A message on its way; `startsTurn` when it is a steer that starts the turn,
   * which reads as the user's next message.
   */
  { type: 'pending_submission'; id: string; submission: PendingSubmissionState; startsTurn: boolean } |
  /**
   * The message the running turn started with, which the server holds and the
   * turn has not written yet: the user's message, as it will read.
   */
  { type: 'starting_message'; id: string; message: QueuedMessage } |
  /**
   * The edge of a window that does not reach its transcript's start or end,
   * which shows its next page coming.
   */
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

/** What Send on the row does, for a row `waitingAs` says waits; undefined for any other. */
export function waitingSend(row: MessageListBlock): WaitingSend | undefined {
  const as = waitingAs(row)
  const id = row.type === 'pending_steer' ? row.pendingSteerId
    : row.type === 'queued_message' ? row.queueId
      : row.type === 'pending_submission' ? row.submission.id
        : undefined
  return as === undefined || id === undefined ? undefined : { as, id }
}
