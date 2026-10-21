import type { Block, PendingCall, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { nonEmptyString } from '@demicodes/utils'
import { pendingSteersToRenderBlocks, type MessageListBlock, type PendingCallRenderBlock } from './pending-steers'
import { queuedMessagesToRenderBlocks } from './queued-messages'
import { turnStartingMessage } from './send-way'
import type { PendingSteerMessage, PendingSubmissionState } from './types'

// The end of a running conversation changes in the frame that causes it, so the
// list never twitches.
//
// - The list only grows, except where the user's own action shrinks it
//   (folding, editing, deleting) or asks for the change (Continue, Resume,
//   Retry).
// - A row that gives way to what comes next rolls over it in place (`RowRoll`)
//   instead of leaving while another appears.
// - Messages that wait, the pending steers and the queue, stand below the turn
//   and never in its flow. What the turn is doing is read from the transcript
//   alone, so the tail row (Requesting or Connecting) stands right after the
//   transcript, above them.
// - The Requesting row says one word for as long as the provider is asked,
//   since which part of the system holds a message is no concern of the person
//   waiting. A row that changed its word as Demi confirmed the message, or with
//   each retry, would show two names for one wait.

/**
 * A compaction in progress: its divider, at the end of the transcript, where
 * the pass appends the marker that shows the finished one.
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
  /** The message the running turn started with, until the turn writes it. */
  starting?: QueuedMessage | null
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSubmissions?: readonly PendingSubmissionState[]
  /**
   * The waiting messages whose Recall is on its way, by id: their rows are
   * gone from the press.
   */
  removing?: readonly string[]
  /**
   * The messages not delivered that stand elsewhere than at the end, by id
   * (`undeliveredPlaces`): before a message sent after them, or outside the
   * window shown.
   */
  elsewhere?: ReadonlySet<string>
}

/**
 * What the list shows after the transcript, in two parts the tail row stands
 * between.
 */
export interface ListTail {
  /**
   * The turn's own rows: a compaction in progress, the calls the model is
   * writing, and a message sent as a message, which is the user's next
   * message, so the tail row's Requesting stands under it; so is a steer
   * that starts the turn.
   */
  turn: MessageListBlock[]
  /**
   * The messages that wait below the turn: the steers, then the queue under its
   * divider, each with the messages on their way that will be ones of them,
   * shown from their send as what they will be, in the order they were sent.
   */
  waiting: MessageListBlock[]
}

/**
 * What the list shows after the transcript.
 *
 * - Steers that arrive during a compaction wait outside what it summarizes, so
 *   they follow its divider.
 * - A message the session holds already shows there alone, in the transcript,
 *   as a steer, in the queue or as the turn's starting message until the turn
 *   writes it. The row it was waiting in becomes that one
 *   (`row-key.ts`), so it never shows twice, whichever of the two the server
 *   tells first.
 * - A message whose Recall is on its way shows nowhere.
 * - A message whose delivery failed is followed by the row that tells it.
 * - A steer on its way that starts the turn, as when nothing runs and the
 *   message before it was not delivered, is the user's next message. It stands
 *   in the turn's flow, in the row it keeps.
 * - A message not delivered stands here, at the end of what it was sent as,
 *   only while no message sent after it shows (`undeliveredPlaces`).
 */
export function listTailBlocks(input: ListTailInput): ListTail {
  const written = new Set(input.blocks.flatMap((block) =>
    block.type === 'user' ? [block.turnId] : block.type === 'steer' ? [block.id] : []))
  const removing = new Set(input.removing)
  const steers = input.pendingSteers.filter((steer) => !written.has(steer.id) && !removing.has(steer.id))
  const queue = input.queue.filter((message) => !written.has(message.id) && !removing.has(message.id))
  const started = input.starting && !written.has(input.starting.id) ? input.starting : null
  const held = new Set([
    ...written,
    ...input.pendingSteers.map((steer) => steer.id),
    ...input.queue.map((message) => message.id),
    ...(input.starting ? [input.starting.id] : []),
  ])
  const sending = (input.pendingSubmissions ?? []).filter((submission) =>
    !held.has(submission.id) && !removing.has(submission.id) && !input.elsewhere?.has(submission.id))
  const starting = turnStartingMessage(input.phase, input.pendingSubmissions ?? [])
  const startsTurn = (submission: PendingSubmissionState) =>
    submission.way === 'steer' && submission.id === starting?.id
  const rows = (shows: (submission: PendingSubmissionState) => boolean): MessageListBlock[] => sending
    .filter(shows)
    .flatMap((submission) => submissionRows(submission, startsTurn(submission)))
  const queued = [...queuedMessagesToRenderBlocks(queue), ...rows((submission) => submission.way === 'queue')]
  return {
    turn: [
      ...(input.phase === 'compacting'
        ? [{ type: 'compaction_progress', id: 'compaction-progress' } as const]
        : []),
      ...(started ? [{ type: 'starting_message', id: `starting:${started.id}`, message: started } as const] : []),
      ...pendingCallsToRenderBlocks(input.pendingCalls, input.blocks),
      ...rows((submission) => submission.way === 'send' || startsTurn(submission)),
    ],
    waiting: [
      ...pendingSteersToRenderBlocks(steers),
      ...rows((submission) => submission.way === 'steer' && !startsTurn(submission)),
      ...(queued.length > 0
        ? [{ type: 'queue_divider', id: 'queue-divider', count: queued.filter((row) => row.type !== 'delivery_failure').length } as const, ...queued]
        : []),
    ],
  }
}

/**
 * The rows of a message on its way: the message, and under it, when its
 * delivery failed, the row that tells it, which the Requesting row gives
 * way to. `startsTurn` when it is a steer that starts the turn.
 */
export function submissionRows(submission: PendingSubmissionState, startsTurn = false): MessageListBlock[] {
  return [
    { type: 'pending_submission', id: `pending-submission:${submission.id}`, submission, startsTurn },
    ...(submission.error !== null
      ? [{ type: 'delivery_failure', id: `delivery-failure:${submission.id}`, submission } as const]
      : []),
  ]
}

/**
 * Where a message not delivered stands: where it was sent, before the first
 * message sent after it that has a row, in the transcript, among the steers, in
 * the queue or on its way.
 */
export type UndeliveredPlace =
  /** Before the row of the message `before`. */
  | { before: string }
  /**
   * At the end of what it was sent as, since no message sent after it shows.
   */
  | 'end'
  /**
   * Outside the window shown: a message sent after it is earlier or later than
   * the window.
   */
  | 'outside'

export interface UndeliveredPlacesInput {
  /** The messages sent and not yet confirmed, in the order they were sent. */
  submissions: readonly PendingSubmissionState[]
  /**
   * The messages that have a row of their own in the list, by the id they were
   * sent with.
   */
  shown: ReadonlySet<string>
  /**
   * The waiting messages whose Recall is on its way, by id, which show nowhere.
   */
  removing: ReadonlySet<string>
  /** The window reaches the transcript's end. */
  latest: boolean
  /** The window reaches the transcript's start. */
  atStart: boolean
}

/**
 * The place of each message not delivered, by its id. Its place is before the
 * first message sent after it that shows.
 *
 * - One taken back, or one not delivered either, is passed over.
 * - A message the session took that shows nowhere is in the transcript outside
 *   the window, unless the window holds the whole transcript, where it is one
 *   taken back elsewhere and is passed over too; so is the message not
 *   delivered, which shows when its window does.
 * - In the latest window, one with no message after it that shows stands at the
 *   end. Any other window holds only those it can place.
 */
export function undeliveredPlaces(input: UndeliveredPlacesInput): Map<string, UndeliveredPlace> {
  const undelivered = input.submissions.filter((submission) => submission.error !== null && !input.removing.has(submission.id))
  const passed = new Set(undelivered.map((submission) => submission.id))
  const whole = input.latest && input.atStart
  return new Map(undelivered.map((submission): [string, UndeliveredPlace] => {
    for (const next of submission.followers ?? []) {
      if (input.removing.has(next) || passed.has(next)) {
        continue
      }
      if (input.shown.has(next)) {
        return [submission.id, { before: next }]
      }
      if (!whole) {
        return [submission.id, 'outside']
      }
    }
    return [submission.id, input.latest ? 'end' : 'outside']
  }))
}

/**
 * The calls the model is writing, each as its row from the moment its
 * description is written, without a call whose block the transcript holds
 * already: the block takes its place, so the call never shows twice. A call
 * without its description yet adds nothing, so the list goes on showing what
 * it showed, Requesting or the step before, rather than a placeholder title.
 */
export function pendingCallsToRenderBlocks(
  calls: readonly PendingCall[],
  blocks: readonly Block[],
): PendingCallRenderBlock[] {
  if (calls.length === 0) {
    return []
  }
  const held = new Set(blocks.flatMap((block) => (block.type === 'tool_call' ? [block.toolUseId] : [])))
  return calls.flatMap((call) => {
    const description = nonEmptyString(call.description)
    return description === undefined || held.has(call.toolUseId)
      ? []
      : [{ type: 'pending_call', id: `pending-call:${call.toolUseId}`, call: { ...call, description } }]
  })
}
