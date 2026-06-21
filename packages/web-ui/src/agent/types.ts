import type { Block, QueuedMessage, SessionPhase, UserContentBlock } from '@demi/core'

export interface ModelIntent {
  providerType: string
  modelId: string
  thinkingEffort: string | null
  serviceTierId: string | null
}

export interface ConversationDraft {
  inputModel: unknown | null
  attachments: UserContentBlock[]
}

/**
 * Reactive per-conversation state. Mirrors the shape agent-gui exposed via
 * `rpc.agent.$state.sessions[id]` so ported components read it the same way.
 */
export interface ConversationState {
  id: string
  cwd: string
  title: string
  blocks: Block[]
  phase: SessionPhase
  queue: QueuedMessage[]
  model: ModelIntent
  draft: ConversationDraft | null
  isResultSeen: boolean
  hasContent: boolean
  lastError: string | null
}
