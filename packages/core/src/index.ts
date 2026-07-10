// Shared data contracts for every Demi package: content blocks, model
// selection, transcript blocks, and session state. No runtime behavior.

// ── content blocks ─────────────────────────────────────────────────

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

// ── model media (closed set) ────────────────────────────────────────
//
// The only binary types a model can receive natively. Detection is by magic
// bytes over this closed set — never open-ended content-type guessing.

export type ModelMediaKind = 'image' | 'video'

export interface ModelMediaType {
  mediaType: string
  kind: ModelMediaKind
  extension: ImageFileExtension | VideoFileExtension
}

const MODEL_MEDIA_TYPES: readonly ModelMediaType[] = [
  { mediaType: 'image/png', kind: 'image', extension: 'png' },
  { mediaType: 'image/jpeg', kind: 'image', extension: 'jpeg' },
  { mediaType: 'image/gif', kind: 'image', extension: 'gif' },
  { mediaType: 'image/webp', kind: 'image', extension: 'webp' },
  { mediaType: 'video/mp4', kind: 'video', extension: 'mp4' },
  { mediaType: 'video/x-m4v', kind: 'video', extension: 'm4v' },
  { mediaType: 'video/quicktime', kind: 'video', extension: 'mov' },
  { mediaType: 'video/webm', kind: 'video', extension: 'webm' },
]

export function modelMediaTypeFor(mediaType: string): ModelMediaType | null {
  return MODEL_MEDIA_TYPES.find((entry) => entry.mediaType === mediaType) ?? null
}

/** Whether this model accepts the given media type natively (per its catalog extensions). */
export function modelAcceptsMediaType(model: Model, mediaType: string): boolean {
  const entry = modelMediaTypeFor(mediaType)
  if (!entry) return false
  if (model.acceptedExtensions.includes(entry.extension)) return true
  // jpg/jpeg are the same format under two extension spellings.
  return entry.extension === 'jpeg' && model.acceptedExtensions.includes('jpg')
}

/**
 * Detect a model-media type from a byte stream's magic numbers. Returns null
 * for anything outside the closed set — callers must not guess further.
 */
export function sniffModelMediaType(bytes: Uint8Array): ModelMediaType | null {
  if (bytes.length < 12) return null
  const at = (index: number): number => bytes[index] ?? 0
  const ascii = (start: number, length: number): string => {
    let out = ''
    for (let i = start; i < start + length && i < bytes.length; i += 1) out += String.fromCharCode(at(i))
    return out
  }

  if (at(0) === 0x89 && ascii(1, 3) === 'PNG') return modelMediaTypeFor('image/png')
  if (at(0) === 0xff && at(1) === 0xd8 && at(2) === 0xff) return modelMediaTypeFor('image/jpeg')
  if (ascii(0, 6) === 'GIF87a' || ascii(0, 6) === 'GIF89a') return modelMediaTypeFor('image/gif')
  if (ascii(0, 4) === 'RIFF' && ascii(8, 4) === 'WEBP') return modelMediaTypeFor('image/webp')
  // EBML header — Matroska family; WebM is the model-relevant container.
  if (at(0) === 0x1a && at(1) === 0x45 && at(2) === 0xdf && at(3) === 0xa3) return modelMediaTypeFor('video/webm')
  if (ascii(4, 4) === 'ftyp') {
    const brand = ascii(8, 4)
    if (brand === 'qt  ') return modelMediaTypeFor('video/quicktime')
    if (brand.startsWith('M4V')) return modelMediaTypeFor('video/x-m4v')
    return modelMediaTypeFor('video/mp4')
  }
  return null
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
      /**
       * Bounded UI-facing enhancement data, typed by the tool's owning layer.
       * Never replayed to the model. Must not embed unbounded payloads (full
       * command output, file bodies, raw or base64 bytes) — anything unbounded
       * lives in a command artifact or blob and is referenced by id.
       */
      view: unknown | null
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
