import type { ContextUsage, InstructionEntry, PendingCall, PendingSteer, ProviderFailureFacts, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import type { HeldTranscript } from '@demicodes/conversation-client'
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
export type PendingSteerMessage = Pick<PendingSteer, 'id' | 'content'>

/** A message sent but not yet confirmed: the composer's own text and attachments, shown as the message they will become. */
export interface PendingSubmissionState {
  id: string
  /**
   * How it was sent, which says what it will be: the next message, a steer
   * waiting for the running turn, or a message in the queue
   * (`product.md` § Steer or queue).
   */
  way: SendWay
  text: string
  attachments: ComposerAttachment[]
  /** Why the delivery failed; null while the message is on its way. */
  error: string | null
}

/** One conversation's live state, as the runtime keeps it and `ChatSession` reads it. */
export interface ConversationState {
  id: string
  cwd: string
  title: string
  /** The parts of the root's transcript the page holds (`web-application.md` § Transcript windows). */
  history: HeldTranscript
  /** The block whose window the transcript shows; null for the latest. */
  shownAt: string | null
  /**
   * The entries of the root's newest instructions block as a page of the
   * transcript named them, for when the page does not hold that block.
   */
  instructions: InstructionEntry[]
  /** Each compaction marker's boundary's summary size, by the marker's id, as pages named them. */
  summaries: Record<string, number>
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  /** The calls the model is writing, as the server lists them (`runtime.md` § Calls being written). */
  pendingCalls: PendingCall[]
  /**
   * When the root began to wait for the model's answer, as `Date.now()`, as
   * the server tells it; null when it waits for none (`runtime.md` § Waiting
   * for the model).
   */
  waitingSince: number | null
  /** The conversation's model settings, which its composer shows. */
  model: ModelSettings
  lastError: string | null
  /** History restore and the live socket. A new conversation starts `ready`. */
  load: SessionLoad
  /** A recovery sent and not yet acknowledged by a `phase` event; the tail row says Requesting meanwhile. */
  pendingAction: PendingAction
  /**
   * What the providers read out of the error blocks' failure records, by block
   * id, as the backend sends them beside the transcript.
   */
  failures: Record<string, ProviderFailureFacts>
  /**
   * How full the next request is, as the backend estimates it
   * (`compaction.md` § Context estimate); null until the session says.
   */
  contextUsage: ContextUsage | null
}

/** What `ChatSession` reads: the live conversation fields plus what the product keeps beside them. */
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
    | 'pendingCalls'
    | 'waitingSince'
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
}
