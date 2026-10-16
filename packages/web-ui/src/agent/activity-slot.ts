import type { SessionPhase } from '@demicodes/protocol'
import type { MessageListBlock } from './pending-steers'
import type { SessionLoad } from './session-status'

/**
 * Why the transcript's tail row is waiting. `connecting` is the socket;
 * `requesting` is the wait for the provider's answer: from the send of a
 * message that starts a turn, its delivery included, and a recovery the server
 * has not acknowledged yet and the agent's own retries, which are the same
 * wait and keep the same word (`turns.md` § Recovering an unfinished turn).
 */
export type ActivityKind = 'connecting' | 'requesting'

/**
 * An action the client sent whose acknowledgement (the next `phase` event)
 * has not arrived. The tail row says Requesting meanwhile.
 */
export type PendingAction = 'resume' | null

/** Tail blocks after which a running turn is waiting for the model. */
const WAITING_TAIL_TYPES: ReadonlySet<MessageListBlock['type']> = new Set([
  'user',
  'steer',
  'agent_message',
  'wakeup',
  'compaction_marker',
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
  /**
   * A recovery this page asked for has not moved past the record it
   * continues from: the server has not acknowledged it, or the turn it runs
   * has shown nothing after the cut yet.
   */
  recovering: boolean
  /**
   * A message sent to start a turn is on its way to Demi: the wait for its
   * answer has begun, though the server has confirmed neither the message nor
   * the turn yet.
   */
  startingTurn: boolean
  /** The visible transcript. */
  transcriptBlocks: readonly MessageListBlock[]
  /**
   * The transcript's rows as the list draws them, the runs of steps grouped,
   * with the calls the model is writing; never the messages that wait or
   * the message being sent, since what the turn is doing is read from the
   * transcript alone (`turns.md` § Following the agent's work).
   */
  turnRows?: readonly MessageListBlock[]
}

/**
 * The tail row's kind, or null when the tail is content (streaming thinking or
 * text, a call being written or executing) or nothing is happening. Text the
 * model finished, with nothing after it yet, is followed by Requesting
 * (`turns.md` § Recovering an unfinished turn). A lost socket wins over
 * everything, since a recovery or a turn cannot progress without it: the row
 * says Connecting while the page still reaches the backend, and nothing while
 * the connection banner says the backend is away. A conversation that failed
 * to open waits for nothing: the failure told at the tail says so. A first
 * opening whose socket is not made yet says Connecting as a lost one does.
 */
export function activitySlotKind(input: ActivitySlotInput): ActivityKind | null {
  if (input.load === 'reconnecting' || input.load === 'connecting') {
    return input.backendAway ? null : 'connecting'
  }
  if (input.load === 'failed') {
    return null
  }
  if (input.recovering || input.startingTurn) {
    return 'requesting'
  }
  return isWaitingForProvider(input) ? 'requesting' : null
}

/** A running turn with nothing streaming and a tail that waits for the model's next output. */
function isWaitingForProvider(input: ActivitySlotInput): boolean {
  if (input.phase !== 'running' || hasActiveOutput(input.transcriptBlocks)) {
    return false
  }
  const last = (input.turnRows ?? input.transcriptBlocks).at(-1)
  if (!last) {
    return true
  }
  if (last.type === 'tool_call') {
    return last.status !== 'executing'
  }
  if (last.type === 'text') {
    return last.forkable === true
  }
  // A run's row holds only steps that ended; a step that still runs is a
  // row of its own after it (`runtime.md` § Work groups). So a run at the
  // tail is what has happened, and the turn waits for the model under it.
  if (last.type === 'work_group') {
    return true
  }
  return WAITING_TAIL_TYPES.has(last.type)
}

/**
 * Whether the turn's newest output still streams, or a call still waits for
 * its result, wherever it stands: the turn waits for that call, not for the
 * model, as parallel calls return in any order (`runtime.md` § Work groups).
 */
function hasActiveOutput(blocks: readonly MessageListBlock[]): boolean {
  const last = blocks[blocks.length - 1]
  return (
    last?.type === 'thinking'
    || (last?.type === 'text' && !last.forkable)
    || blocks.some((block) => block.type === 'tool_call' && block.status === 'executing')
  )
}
