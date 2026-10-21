import type { ContextUsage, InstructionEntry, MessageComment, PendingCall, PendingSteer, ProviderFailureFacts, QueuedMessage, SessionPhase, TurnSpeed } from '@demicodes/protocol'
import type { ShownError } from '../infra/errors'
import type { HeldTranscript, RequestOutputReading } from '@demicodes/conversation-client'
import type { PendingAction } from './activity-slot'
import type { SessionLoad } from './session-status'
import type { SubagentRecord } from './subagents'
import type { TerminalRecord } from './terminals'
import type { ComposerAttachment } from './message-input/attachments'
import type { PersistedScrollState } from '../composables/useBlockVirtualizer'
import type { ModelSettings } from './model-selection'
import type { SendWay } from './send-way'

export type { ModelSettings } from './model-selection'

/** A steer the server holds for the running turn, as it sends them. */
export type PendingSteerMessage = Pick<PendingSteer, 'id' | 'content' | 'comments'>

/**
 * A message sent but not yet confirmed: the composer's own text and
 * attachments, shown as the message they will become.
 */
export interface PendingSubmissionState {
  id: string
  /**
   * How it was sent, which says what it will be: the next message, a steer
   * waiting for the running turn, or a message in the queue.
   */
  way: SendWay
  text: string
  attachments: ComposerAttachment[]
  /** The comments it carries; absent with none. */
  comments?: readonly MessageComment[]
  /** Why the delivery failed; null while the message is on its way. */
  error: string | null
  /**
   * The request ID of the refusal, shown beside `error`; null or absent with
   * none.
   */
  requestId?: string | null
  /**
   * False for a refusal that sending again cannot change, such as a file of
   * the message that is gone: the dock's Retry skips it, and its Edit is the
   * way on. Absent otherwise.
   */
  retryable?: false
  /**
   * The ids of the messages sent after it, a turn's, a steer's or a queued
   * message's, in the order they were sent: a message not delivered stands
   * before the first of them that shows. Absent or empty while none was.
   */
  followers?: readonly string[]
}

/**
 * One conversation's live state, as the runtime keeps it and `ChatSession`
 * reads it.
 */
export interface ConversationState {
  id: string
  cwd: string
  /**
   * Null for the placeholder, which shows as New conversation in the reader's
   * language.
   */
  title: string | null
  /** The parts of the root's transcript the page holds. */
  history: HeldTranscript
  /** The block whose window the transcript shows; null for the latest. */
  shownAt: string | null
  /**
   * The entries of the root's newest instructions block as a page of the
   * transcript named them, for when the page does not hold that block.
   */
  instructions: InstructionEntry[]
  /**
   * Each compaction marker's boundary's summary size, by the marker's id, as
   * pages named them.
   */
  summaries: Record<string, number>
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  /**
   * The message the root's running turn started with, until the turn writes it
   * into the transcript, as the server tells it; null otherwise.
   */
  starting: QueuedMessage | null
  /** The calls the model is writing, as the server lists them. */
  pendingCalls: PendingCall[]
  /**
   * When the root began to wait for the model's answer, as `Date.now()`, as
   * the server tells it; null when it waits for none.
   */
  waitingSince: number | null
  /** The output of the request the root agent runs, as the server tells it; null when its turn has none. */
  requestOutput: RequestOutputReading | null
  /** The conversation's model settings, which its composer shows. */
  model: ModelSettings
  lastError: ShownError | null
  /** History restore and the live socket. A new conversation starts `ready`. */
  load: SessionLoad
  /**
   * A recovery sent and not yet acknowledged by a `phase` event; the tail row
   * says Requesting meanwhile.
   */
  pendingAction: PendingAction
  /**
   * What the providers read out of the error blocks' failure records, by block
   * id, as the backend sends them beside the transcript.
   */
  failures: Record<string, ProviderFailureFacts>
  /**
   * How full the root's next request is, as the backend estimates it;
   * null until the session says. A subagent's is its record's.
   */
  contextUsage: ContextUsage | null
}

/**
 * What `ChatSession` reads: the live conversation fields plus what the product
 * keeps beside them.
 */
export interface ChatSessionState
  extends Pick<
    ConversationState,
    | 'id'
    | 'cwd'
    | 'title'
    | 'history'
    | 'shownAt'
    | 'summaries'
    | 'queue'
    | 'pendingSteers'
    | 'starting'
    | 'pendingCalls'
    | 'waitingSince'
    | 'requestOutput'
    | 'phase'
    | 'load'
    | 'lastError'
    | 'pendingAction'
    | 'failures'
  > {
  archived: boolean
  scroll: PersistedScrollState | null
  subagents: SubagentRecord[]
  terminals: TerminalRecord[]
  /** The speed of the root's last turn, as the conversation's metadata brings it; null before its first. */
  turnSpeed: TurnSpeed | null
}
