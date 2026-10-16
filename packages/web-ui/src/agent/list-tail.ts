import type { Block, PendingCall, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { nonEmptyString } from '@demicodes/utils'
import { pendingSteersToRenderBlocks, type MessageListBlock, type PendingCallRenderBlock } from './pending-steers'
import { queuedMessagesToRenderBlocks } from './queued-messages'
import { turnStartingMessage } from './send-way'
import type { PendingSteerMessage, PendingSubmissionState } from './types'

/**
 * A compaction in progress: its divider, at the end of the transcript, where
 * the pass appends the marker that shows the finished one
 * (`transcript.md` § Block types).
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
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSubmissions?: readonly PendingSubmissionState[]
  /**
   * The waiting messages whose Recall is on its way, by id: their rows are
   * gone from the press (`turns.md` § Steer or queue).
   */
  removing?: readonly string[]
}

/** What the list shows after the transcript, in two parts the tail row stands between. */
export interface ListTail {
  /**
   * The turn's own rows: a compaction in progress, the calls the model is
   * writing, and a message sent as a message, which is the user's next
   * message, so the tail row's Requesting stands under it; so is a steer
   * that starts the turn.
   */
  turn: MessageListBlock[]
  /**
   * The messages that wait below the turn (`turns.md` § Following the
   * agent's work): the steers, then the queue under its divider, each with
   * the messages on their way that will be ones of them, shown from their
   * send as what they will be, in the order they were sent.
   */
  waiting: MessageListBlock[]
}

/**
 * What the list shows after the transcript. Steers that arrive during a
 * compaction wait outside what it summarizes, so they follow its divider. A
 * message the session holds already shows there alone, in the transcript, as
 * a steer or in the queue: the row it was waiting in becomes that one
 * (`row-key.ts`), so it never shows twice, whichever of the two the server
 * tells first. A message whose Recall is on its way shows nowhere. A message
 * whose delivery failed is followed by the row that tells it. A steer on its
 * way that starts the turn, as when nothing runs and the message before it
 * was not delivered, is the user's next message (`turns.md` § Following
 * the agent's work): it stands in the turn's flow, in the row it keeps.
 */
export function listTailBlocks(input: ListTailInput): ListTail {
  const written = new Set(input.blocks.flatMap((block) =>
    block.type === 'user' ? [block.turnId] : block.type === 'steer' ? [block.id] : []))
  const removing = new Set(input.removing)
  const steers = input.pendingSteers.filter((steer) => !written.has(steer.id) && !removing.has(steer.id))
  const queue = input.queue.filter((message) => !written.has(message.id) && !removing.has(message.id))
  const held = new Set([...written, ...input.pendingSteers.map((steer) => steer.id), ...input.queue.map((message) => message.id)])
  const sending = (input.pendingSubmissions ?? []).filter((submission) =>
    !held.has(submission.id) && !removing.has(submission.id))
  const starting = turnStartingMessage(input.phase, input.pendingSubmissions ?? [])
  const startsTurn = (submission: PendingSubmissionState) =>
    submission.way === 'steer' && submission.id === starting?.id
  // A message whose delivery failed is told under it, as a row of its own:
  // the row Requesting gives way to, and which Retry rolls back into it.
  const rows = (shows: (submission: PendingSubmissionState) => boolean): MessageListBlock[] => sending
    .filter(shows)
    .flatMap((submission): MessageListBlock[] => [
      { type: 'pending_submission', id: `pending-submission:${submission.id}`, submission, startsTurn: startsTurn(submission) },
      ...(submission.error !== null
        ? [{ type: 'delivery_failure', id: `delivery-failure:${submission.id}`, submission } as const]
        : []),
    ])
  const queued = [...queuedMessagesToRenderBlocks(queue), ...rows((submission) => submission.way === 'queue')]
  return {
    turn: [
      ...(input.phase === 'compacting'
        ? [{ type: 'compaction_progress', id: 'compaction-progress' } as const]
        : []),
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
 * The calls the model is writing, each as its row from the moment its
 * description is written, without a call whose block the transcript holds
 * already: the block takes its place, so the call never shows twice
 * (`rendering.md` § Calls being written). A call without its description yet
 * adds nothing, so the list goes on showing what it showed, Requesting or
 * the step before, rather than a placeholder title.
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
