import type { SessionPhase } from '@demicodes/protocol'
import { CHROME_ROLL_MS } from '../ui/chrome-roll'
import type { MessageListBlock } from './pending-steers'
import { isQueueTailBlock } from './queued-messages'
import type { SessionLoad } from './session-status'

/**
 * Why the transcript's tail row is waiting. `connecting` is the socket;
 * `requesting` is the wait for the provider's answer: from the send of a
 * message that starts a turn, its delivery included, and a recovery the server
 * has not acknowledged yet and the agent's own retries, which are the same
 * wait and keep the same word (`product.md` § Recovering an unfinished turn).
 */
export type ActivityKind = 'connecting' | 'requesting'

/**
 * An action the client sent whose acknowledgement (the next `phase` event)
 * has not arrived. The tail row says Requesting meanwhile.
 */
export type PendingAction = 'resume' | null

/** A block that enters the transcript through the activity slot: it rolls in as the slot's face, then the row becomes the block. */
export type HandoffBlock = Extract<MessageListBlock, { type: 'thinking' | 'tool_call' | 'pending_call' }>

/** The face roll, then a beat before the transcript row takes over. */
export const ACTIVITY_HANDOFF_MS = CHROME_ROLL_MS + 80

/** Tail blocks after which a running turn is waiting for the model; an abort stays visible until the server marks it resumed. */
const WAITING_TAIL_TYPES: ReadonlySet<MessageListBlock['type']> = new Set([
  'user',
  'steer',
  'agent_message',
  'wakeup',
  'pending_steer',
  'compaction_marker',
  'abort',
])

export interface ActivitySlotInput {
  load: SessionLoad
  /**
   * The page cannot reach the backend, and the host's connection banner says
   * so (`web-application.md` § A page of another build). A conversation whose
   * socket waits for the same backend leaves that to the banner.
   */
  backendAway: boolean
  phase: SessionPhase
  pendingAction: PendingAction
  /**
   * A message sent to start a turn is on its way to Demi: the wait for its
   * answer has begun, though the server has confirmed neither the message nor
   * the turn yet.
   */
  startingTurn: boolean
  /** The visible transcript, including a record a pending recovery hides from the list. */
  transcriptBlocks: readonly MessageListBlock[]
  /** What the list renders: the transcript with pending steers and the queue after it. */
  renderBlocks?: readonly MessageListBlock[]
}

/**
 * The tail row's kind, or null when the tail is content (streaming thinking or
 * text, a call being written or executing) or nothing is happening. Text the
 * model finished, with nothing after it yet, is followed by Requesting
 * (`product.md` § Recovering an unfinished turn). A lost socket wins over
 * everything, since a recovery or a turn cannot progress without it: the row
 * says Connecting while the page still reaches the backend, and nothing while
 * the connection banner says the backend is away.
 */
export function activitySlotKind(input: ActivitySlotInput): ActivityKind | null {
  if (input.load === 'reconnecting') {
    return input.backendAway ? null : 'connecting'
  }
  if (input.pendingAction === 'resume' || input.startingTurn) {
    return 'requesting'
  }
  return isWaitingForProvider(input) ? 'requesting' : null
}

/** A running turn with nothing streaming and a tail that waits for the model's next output. */
function isWaitingForProvider(input: ActivitySlotInput): boolean {
  if (input.phase !== 'running' || hasActiveOutput(input.transcriptBlocks)) {
    return false
  }
  const last = lastNonQueueBlock(input.renderBlocks ?? input.transcriptBlocks)
  if (!last) {
    return true
  }
  if (last.type === 'tool_call') {
    return last.status !== 'executing'
  }
  if (last.type === 'text') {
    return last.forkable === true
  }
  // A running group's face is the newest step: the steps after it roll over
  // it there, not into the tail row.
  if (last.type === 'work_group') {
    return !last.live
  }
  return WAITING_TAIL_TYPES.has(last.type)
}

export function isHandoffBlock(block: MessageListBlock | undefined): block is HandoffBlock {
  return block?.type === 'thinking' || block?.type === 'tool_call' || block?.type === 'pending_call'
}

function lastNonQueueBlock(blocks: readonly MessageListBlock[]): MessageListBlock | undefined {
  for (let i = blocks.length - 1; i >= 0; i--) {
    const block = blocks[i]
    if (block && !isQueueTailBlock(block) && block.type !== 'pending_submission') {
      return block
    }
  }
}

function hasActiveOutput(blocks: readonly MessageListBlock[]): boolean {
  const last = blocks[blocks.length - 1]
  return (
    last?.type === 'thinking'
    || (last?.type === 'text' && !last.forkable)
    || (last?.type === 'tool_call' && last.status === 'executing')
  )
}
