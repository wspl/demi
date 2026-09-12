import type { SessionPhase } from '@demicodes/core'
import { CHROME_ROLL_MS } from '../ui/chrome-roll'
import type { MessageListBlock } from './pending-steers'
import { isQueueTailBlock } from './queued-messages'
import type { SessionLoad } from './session-status'

/**
 * Why the transcript's tail row is waiting. `connecting` is the socket;
 * `resuming` and `retrying` are a recovery the server has not acknowledged
 * yet; `requesting` is a running turn between outputs.
 */
export type ActivityKind = 'connecting' | 'resuming' | 'retrying' | 'requesting'

/**
 * An action the client sent whose acknowledgement (the next `phase` event)
 * has not arrived. The tail row names it meanwhile.
 */
export type PendingAction = 'resume' | null

/** A block that enters the transcript through the activity slot: it rolls in as the slot's face, then the row becomes the block. */
export type HandoffBlock = Extract<MessageListBlock, { type: 'thinking' | 'tool_call' }>

/** The face roll, then a beat before the transcript row takes over. */
export const ACTIVITY_HANDOFF_MS = CHROME_ROLL_MS + 80

/** Tail blocks after which a running turn is waiting for the model; an abort stays visible until the server marks it resumed. */
const WAITING_TAIL_TYPES: ReadonlySet<MessageListBlock['type']> = new Set([
  'user',
  'steer',
  'pending_steer',
  'compaction_boundary',
  'abort',
])

export interface ActivitySlotInput {
  load: SessionLoad
  phase: SessionPhase
  pendingAction: PendingAction
  /** The visible transcript, including a record a pending recovery hides from the list. */
  transcriptBlocks: readonly MessageListBlock[]
  /** What the list renders: the transcript with pending steers and the queue after it. */
  renderBlocks?: readonly MessageListBlock[]
}

/**
 * The tail row's kind, or null when the tail is content (streaming thinking or
 * text, an executing tool) or nothing is happening. Connecting wins over
 * everything: a recovery or a turn cannot progress without the socket.
 */
export function activitySlotKind(input: ActivitySlotInput): ActivityKind | null {
  const renderBlocks = input.renderBlocks ?? input.transcriptBlocks
  if (input.load === 'reconnecting') {
    return 'connecting'
  }
  if (input.pendingAction === 'resume') {
    const tail = input.transcriptBlocks.at(-1)
    return tail?.type === 'error' ? 'retrying' : 'resuming'
  }
  if (input.phase !== 'running') {
    return null
  }
  if (hasActiveOutput(input.transcriptBlocks)) {
    return null
  }
  const last = lastNonQueueBlock(renderBlocks)
  if (!last) {
    return 'requesting'
  }
  if (last.type === 'tool_call') {
    return last.status === 'executing' ? null : 'requesting'
  }
  return WAITING_TAIL_TYPES.has(last.type) ? 'requesting' : null
}

export function isHandoffBlock(block: MessageListBlock | undefined): block is HandoffBlock {
  return block?.type === 'thinking' || block?.type === 'tool_call'
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
    || last?.type === 'text'
    || (last?.type === 'tool_call' && last.status === 'executing')
  )
}
