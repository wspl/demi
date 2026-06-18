// demi core types — 对照 Rust alloy-generated / types.g.ts

// ── 基础内容块 ───────────────────────────────────────────────────────

export interface BlockMeta {
  id: string
  createdAt: string
}

export type ImageSource =
  | { type: 'binary'; data: Uint8Array; mediaType: string }
  | { type: 'url'; url: string }

export interface DocumentSource {
  data: Uint8Array
  mediaType: string
  fileName: string
}

export type UserContentBlock =
  | { type: 'text'; text: string }
  | { type: 'image'; source: ImageSource }
  | { type: 'document'; source: DocumentSource }
  | { type: 'reference'; reference: string }

export interface Base64ImageSource {
  mediaType: string
  data: string
}

export type ToolResultContentBlock =
  | { type: 'text'; text: string }
  | { type: 'image'; source: Base64ImageSource }

// ── token / model ───────────────────────────────────────────────────

export interface TokenUsage {
  inputTokens: number
  outputTokens: number
  cacheReadTokens: number
  cacheWriteTokens: number
}

export type FileExtension = 'png' | 'jpg' | 'jpeg' | 'gif' | 'webp' | 'pdf'

export type ThinkingEffort =
  | 'none'
  | 'minimal'
  | 'low'
  | 'medium'
  | 'high'
  | 'max'
  | 'xhigh'

export type ThinkingSummary = 'auto' | 'concise' | 'detailed' | 'off' | 'on'

export type ThinkingCapability =
  | { type: 'adaptive'; efforts: ThinkingEffort[]; defaultEffort: ThinkingEffort | null }
  | {
      type: 'budget'
      minBudgetTokens: number | null
      maxBudgetTokens: number | null
      defaultBudgetTokens: number | null
    }
  | {
      type: 'effort'
      efforts: ThinkingEffort[]
      defaultEffort: ThinkingEffort | null
      summaries: ThinkingSummary[]
      defaultSummary: ThinkingSummary | null
    }
  | { type: 'disabled' }

export type ThinkingConfig =
  | { type: 'adaptive'; effort: ThinkingEffort }
  | { type: 'budget'; budgetTokens: number }
  | { type: 'effort'; effort: ThinkingEffort; summary: ThinkingSummary | null }
  | { type: 'disabled' }

export interface Model {
  id: string
  name: string
  contextWindow: number
  inputLimit: number | null
  thinking: ThinkingCapability[]
  acceptedExtensions: FileExtension[]
}

export interface ModelSelection {
  providerId: string
  model: Model
  thinking: ThinkingConfig | null
}

// ── tool call ───────────────────────────────────────────────────────

export type ToolCallStatus = 'executing' | 'completed' | 'error'

// ── transcript block ────────────────────────────────────────────────

export type Block =
  | {
      type: 'user'
      id: string
      createdAt: string
      model: ModelSelection
      content: UserContentBlock[]
      preamble: string | null
    }
  | { type: 'resume'; id: string; createdAt: string; model: ModelSelection }
  | {
      type: 'thinking'
      id: string
      createdAt: string
      model: ModelSelection
      text: string
      signature: string | null
    }
  | {
      type: 'redacted_thinking'
      id: string
      createdAt: string
      model: ModelSelection
      data: string
    }
  | { type: 'text'; id: string; createdAt: string; model: ModelSelection; text: string }
  | {
      type: 'tool_call'
      id: string
      createdAt: string
      model: ModelSelection
      toolUseId: string
      toolName: string
      input: string
      status: ToolCallStatus
      streamingOutput: ToolResultContentBlock[]
      output: ToolResultContentBlock[]
      metadata: unknown | null
    }
  | {
      type: 'response'
      id: string
      createdAt: string
      model: ModelSelection
      usage: TokenUsage
    }
  | {
      type: 'error'
      id: string
      createdAt: string
      model: ModelSelection
      message: string
      code: string | null
    }
  | { type: 'abort'; id: string; createdAt: string; model: ModelSelection; isResumed: boolean }
  | {
      type: 'compaction_boundary'
      id: string
      createdAt: string
      model: ModelSelection
      summary: string
      summaryTokens: number
    }
  | {
      type: 'compaction_marker'
      id: string
      createdAt: string
      model: ModelSelection
      boundaryId: string
      compactedTokens: number
    }
  | {
      type: 'extension_state_snapshot'
      id: string
      createdAt: string
      extensionName: string
      state: unknown
    }

// ── session ─────────────────────────────────────────────────────────

export type SessionPhase = 'idle' | 'running' | 'compacting'

export interface QueuedMessage {
  id: string
  text: string
  content: UserContentBlock[]
}

export interface Transcript {
  blocks: Block[]
}

export interface AgentSessionCoreState {
  transcript: Transcript
  phase: SessionPhase
  queue: QueuedMessage[]
}
