import type { Block, PendingSteer, ProviderFailureFacts, QueuedMessage, SessionPhase } from '@demicodes/core'
import type { PendingAction } from './activity-slot'
import type { SessionLoad } from './session-status'
import type { SubagentRecord } from './subagents'
import type { TerminalRecord } from './terminals'
import type { ComposerAttachment } from './message-input/attachments'
import type { PersistedScrollState } from '../composables/useBlockVirtualizer'
import type { ModelIntent } from './model-selection'

export type { ModelIntent } from './model-selection'

/** A steer the server holds for the running turn, as it sends them. */
export type PendingSteerMessage = Pick<PendingSteer, 'id' | 'content'>

/** A message sent but not yet confirmed: the composer's own text and attachments, shown as the message they will become. */
export interface PendingSubmissionState {
  id: string
  text: string
  attachments: ComposerAttachment[]
  error: string | null
  sending: boolean
}

/** One conversation's live state, as the runtime keeps it and `ChatSession` reads it. */
export interface ConversationState {
  id: string
  cwd: string
  title: string
  blocks: Block[]
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  model: ModelIntent
  lastError: string | null
  /** History restore and the live socket. A new conversation starts `ready`. */
  load: SessionLoad
  /** A recovery sent and not yet acknowledged by a `phase` event; the tail row says Requesting meanwhile. */
  pendingAction: PendingAction
  /**
   * The agent is retrying a failed provider request on its own: set by
   * `retry_scheduled`, ended by the next transcript change (the retry's
   * output) or the turn's end. The tail row says Retrying meanwhile.
   */
  retrying: boolean
  /**
   * What the providers read out of the error blocks' failure records, by block
   * id, as the backend sends them beside the transcript.
   */
  failures: Record<string, ProviderFailureFacts>
}

/** What `ChatSession` reads: the live conversation fields plus what the product keeps beside them. */
export interface ChatSessionState
  extends Pick<
    ConversationState,
    | 'id'
    | 'cwd'
    | 'title'
    | 'blocks'
    | 'queue'
    | 'pendingSteers'
    | 'phase'
    | 'load'
    | 'lastError'
    | 'pendingAction'
    | 'retrying'
    | 'failures'
  > {
  archived: boolean
  scroll: PersistedScrollState | null
  subagents: SubagentRecord[]
  terminals: TerminalRecord[]
}
