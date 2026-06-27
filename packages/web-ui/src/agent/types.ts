import type { Block, QueuedMessage, SessionPhase, UserContentBlock } from '@demicodes/core'

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
}
