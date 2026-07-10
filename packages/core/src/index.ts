// Shared data contracts for every Demi package: content blocks, model
// selection, transcript blocks, and session state. No runtime behavior.

// ── 基础内容块 ───────────────────────────────────────────────────────

export interface BlockMeta {
  id: string
  createdAt: string
}

export type ImageSource =
  | { type: 'binary'; data: Uint8Array; mediaType: string }
  | { type: 'url'; url: string }

// Native video input (not frame extraction). Only models whose catalog entry marks
// video support accept it; providers reject it otherwise.
export type VideoSource =
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
  | { type: 'video'; source: VideoSource }
  | { type: 'document'; source: DocumentSource }
  | { type: 'reference'; reference: string }

export interface Base64ImageSource {
  mediaType: string
  data: string
}

export interface Base64VideoSource {
  mediaType: string
  data: string
}

export type ToolResultContentBlock =
  | { type: 'text'; text: string }
  | { type: 'image'; source: Base64ImageSource }
  | { type: 'video'; source: Base64VideoSource }

// ── token / model ───────────────────────────────────────────────────

export interface TokenUsage {
  inputTokens: number
  outputTokens: number
  cacheReadTokens: number
  cacheWriteTokens: number
}

/** A `TokenUsage` with every counter at zero. */
export function zeroUsage(): TokenUsage {
  return { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }
}

export type ImageFileExtension = 'png' | 'jpg' | 'jpeg' | 'gif' | 'webp'
export type VideoFileExtension = 'mp4' | 'mov' | 'webm' | 'm4v'
export type FileExtension = ImageFileExtension | VideoFileExtension | 'pdf'

export const VIDEO_FILE_EXTENSIONS: readonly VideoFileExtension[] = ['mp4', 'mov', 'webm', 'm4v']

export type ThinkingEffort = string

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

export function modelAcceptsVideo(model: Model): boolean {
  return VIDEO_FILE_EXTENSIONS.some((extension) => model.acceptedExtensions.includes(extension))
}

/** Media asset kinds a command may emit to this model (images always; video per catalog). */
export function supportedAssetTypesFor(model: Model): ('image' | 'video')[] {
  return modelAcceptsVideo(model) ? ['image', 'video'] : ['image']
}

export interface ModelSelection {
  providerId: string
  model: Model
  thinking: ThinkingConfig | null
  serviceTierId?: string | null
}

// ── tool call ───────────────────────────────────────────────────────

export type ToolCallStatus = 'executing' | 'completed' | 'error'

// ── transcript block ────────────────────────────────────────────────

export type Block =
  | {
      type: 'user'
      id: string
      turnId: string
      createdAt: string
      model: ModelSelection
      content: UserContentBlock[]
      preamble: string | null
      // When true, this user turn is an internal input (e.g. a yield wakeup): replayed to the
      // model like any user_message, but never rendered to the user. Absent/false for real input.
      hidden?: boolean
    }
  | { type: 'resume'; id: string; turnId: string; createdAt: string; model: ModelSelection }
  | {
      type: 'steer'
      id: string
      turnId: string
      createdAt: string
      model: ModelSelection
      content: UserContentBlock[]
      // When true, this steer is an internal input (e.g. a yield wakeup): replayed as user_steer
      // but never rendered. Absent/false for real user steers.
      hidden?: boolean
    }
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
