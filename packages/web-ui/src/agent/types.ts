import type { Block, QueuedMessage, SessionPhase, UserContentBlock } from '@demicodes/core'
import type { PendingAction } from './activity-slot'
import type { ConversationStatus } from './conversation-status'
import type { SessionLoad } from './session-status'
import type { SubagentRecord } from './subagents'
import type { TerminalRecord } from './terminals'
import type { ComposerAttachment } from './message-input/attachments'
import type { PersistedScrollState } from '../composables/useBlockVirtualizer'

export interface ModelIntent {
  providerId: string
  modelId: string
  thinkingEffort: string | null
  serviceTierId: string | null
}

export interface ConversationDraft {
  inputModel: unknown | null
  attachments: UserContentBlock[]
}

export interface PendingSteerMessage {
  id: string
  content: UserContentBlock[]
  baselineSteerBlockIds: string[]
}

/** A message sent but not yet confirmed: the composer's own text and attachments, shown as the message they will become. */
export interface PendingSubmissionState {
  id: string
  text: string
  attachments: ComposerAttachment[]
  error: string | null
  sending: boolean
}

/**
 * Reactive per-conversation state. Mirrors the shape agent-gui exposed via
 * `rpc.agent.$state.sessions[id]` so ported components read it the same way.
 */
export interface ConversationState {
  id: string
  cwd: string
  title: string
  createdAt: string
  blocks: Block[]
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  model: ModelIntent
  draft: ConversationDraft | null
  isResultSeen: boolean
  hasContent: boolean
  lastError: string | null
  /** History restore and the live socket. A new conversation starts `ready`. */
  load: SessionLoad
  /** A recovery sent and not yet acknowledged by a `phase` event; the tail row names it. */
  pendingAction: PendingAction
}

/** What `ChatSession` reads: the live conversation fields plus what the product keeps beside them. */
export interface ChatSessionState
  extends Pick<
    ConversationState,
    | 'id'
    | 'title'
    | 'blocks'
    | 'queue'
    | 'pendingSteers'
    | 'phase'
    | 'load'
    | 'lastError'
    | 'pendingAction'
  > {
  archived: boolean
  status: ConversationStatus
  scroll: PersistedScrollState | null
  subagents: SubagentRecord[]
  terminals: TerminalRecord[]
}
