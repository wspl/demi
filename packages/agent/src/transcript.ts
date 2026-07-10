import { createId, safeJsonStringify } from '@demicodes/utils'
import type {
  Block,
  ModelSelection,
  ToolResultContentBlock,
  Transcript as CoreTranscript,
  UserContentBlock,
} from '@demicodes/core'
import type { InferenceItem, ProviderEvent } from '@demicodes/provider'
import type { TranscriptPatch } from './frames'

const DEFAULT_MODEL_TEXT_HEAD_CHARS = 8_000
const DEFAULT_MODEL_TEXT_TAIL_CHARS = 8_000
// Conservative weights for non-text content in the context estimate. Images
// cost tokens regardless of their text rendering; before these weights an
// image-heavy history estimated near zero and never triggered compaction.
const IMAGE_BASE_TOKENS = 1_600
const IMAGE_BYTES_PER_TOKEN = 1_000
const DOCUMENT_BYTES_PER_TOKEN = 4

export interface TranscriptOptions {
  idFactory?: () => string
  now?: () => string
  /**
   * Per-text-block bounds applied when replaying to the model (head+tail with
   * an elision marker). Defaults to 8000/8000 characters.
   */
  replayTextBounds?: { headChars: number; tailChars: number }
}

export interface CompactionWindow {
  startIndex: number
  cutPoint: number
}

export interface DrainedTranscriptPatches {
  revision: number
  patches: TranscriptPatch[]
}

export class Transcript implements CoreTranscript {
  readonly blocks: Block[]

  private readonly idFactory: () => string
  private readonly now: () => string
  // Mutation journal: every mutating method records the wire patch describing
  // it, so replication never has to diff snapshots. Values are cloned at record
  // time because live blocks keep mutating (streaming text appends).
  private journal: TranscriptPatch[] = []
  private revisionCounter = 0
  private readonly replayHeadChars: number
  private readonly replayTailChars: number

  constructor(blocks: Block[] = [], options: TranscriptOptions = {}) {
    this.blocks = [...blocks]
    this.idFactory = options.idFactory ?? createId
    this.now = options.now ?? (() => new Date().toISOString())
    this.replayHeadChars = options.replayTextBounds?.headChars ?? DEFAULT_MODEL_TEXT_HEAD_CHARS
    this.replayTailChars = options.replayTextBounds?.tailChars ?? DEFAULT_MODEL_TEXT_TAIL_CHARS
  }

  snapshot(): CoreTranscript {
    return { blocks: structuredClone(this.blocks) }
  }

  /** Monotonic revision, advanced once per drained patch batch. */
  get revision(): number {
    return this.revisionCounter
  }

  /**
   * Drains the patches recorded since the last drain, advancing the revision.
   * Returns null when nothing changed.
   */
  takePatches(): DrainedTranscriptPatches | null {
    if (this.journal.length === 0) return null
    this.revisionCounter += 1
    return { revision: this.revisionCounter, patches: this.journal.splice(0) }
  }

  private record(patch: TranscriptPatch): void {
    if (patch.op === 'append_text') {
      const last = this.journal[this.journal.length - 1]
      if (last?.op === 'append_text' && last.path[1] === patch.path[1]) {
        last.delta += patch.delta
        return
      }
    }
    this.journal.push(patch)
  }

  private recordAdd(index: number, block: Block): void {
    this.record({ op: 'add', path: ['blocks', index], value: structuredClone(block) })
  }

  private recordBlockReplace(index: number): void {
    this.record({ op: 'replace_block', path: ['blocks', index], value: structuredClone(this.blocks[index]) })
  }

  private recordReplaceAll(): void {
    // A bulk rewrite invalidates any finer-grained patches recorded before it.
    this.journal = []
    this.record({ op: 'replace', path: ['blocks'], value: structuredClone(this.blocks) })
  }

  pushUserTurn(
    turnId: string,
    model: ModelSelection,
    content: UserContentBlock[],
    preamble: string | null = null,
    hidden = false,
  ): Block {
    const block: Block = {
      type: 'user',
      id: this.idFactory(),
      turnId,
      createdAt: this.now(),
      model,
      content,
      preamble,
      ...(hidden ? { hidden: true } : {}),
    }
    return this.appendBlock(block)
  }

  pushResumeTurn(turnId: string, model: ModelSelection): Block {
    const block: Block = {
      type: 'resume',
      id: this.idFactory(),
      turnId,
      createdAt: this.now(),
      model,
    }
    return this.appendBlock(block)
  }

  pushSteer(
    turnId: string,
    model: ModelSelection,
    content: UserContentBlock[],
    id = this.idFactory(),
    hidden = false,
  ): Block {
    const block: Block = {
      type: 'steer',
      id,
      turnId,
      createdAt: this.now(),
      model,
      content,
      ...(hidden ? { hidden: true } : {}),
    }
    return this.appendBlock(block)
  }

  pushAbort(model: ModelSelection, isResumed = false): Block {
    const block: Block = {
      type: 'abort',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
      isResumed,
    }
    return this.appendBlock(block)
  }

  markLatestAbortResumed(): boolean {
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      const block = this.blocks[i]
      if (block.type === 'abort') {
        block.isResumed = true
        this.recordBlockReplace(i)
        return true
      }
    }
    return false
  }

  /**
   * Rewrites the transcript for a retry: drops everything after the last user
   * turn except that turn's steers (which are replayed to the new attempt).
   * Returns the user block, or null when there is no user turn to retry.
   */
  rewindToLastUserTurn(): Extract<Block, { type: 'user' }> | null {
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      const block = this.blocks[i]
      if (block.type !== 'user') continue
      const preservedSteers = this.blocks
        .slice(i + 1)
        .filter((candidate): candidate is Extract<Block, { type: 'steer' }> => {
          return candidate.type === 'steer' && candidate.turnId === block.turnId
        })
      this.blocks.splice(i + 1, this.blocks.length - i - 1, ...preservedSteers)
      this.recordReplaceAll()
      return block
    }
    return null
  }

  applyProviderEvent(model: ModelSelection, event: ProviderEvent): Block | null {
    switch (event.type) {
      case 'thinking_start':
        return this.appendThinking(model, '')
      case 'thinking_delta':
        return this.appendThinking(model, event.text)
      case 'thinking_signature':
        return this.setLatestThinkingSignature(event.signature)
      case 'redacted_thinking':
        return this.appendBlock({
          type: 'redacted_thinking',
          id: this.idFactory(),
          createdAt: this.now(),
          model,
          data: event.data,
        })
      case 'text_delta':
        return this.appendText(model, event.text)
      case 'tool_call_requested':
        return this.appendBlock({
          type: 'tool_call',
          id: this.idFactory(),
          createdAt: this.now(),
          model,
          toolUseId: event.toolUseId,
          toolName: event.toolName,
          input: stringifyToolInput(event.input),
          status: 'executing',
          streamingOutput: [],
          output: [],
          metadata: null,
        })
      case 'response':
        return this.appendBlock({
          type: 'response',
          id: this.idFactory(),
          createdAt: this.now(),
          model,
          usage: event.usage,
        })
      case 'error':
        return this.appendBlock({
          type: 'error',
          id: this.idFactory(),
          createdAt: this.now(),
          model,
          message: event.message,
          code: event.code,
        })
      case 'abort':
        return this.pushAbort(model)
    }
  }

  completeToolCall(
    toolUseId: string,
    output: ToolResultContentBlock[],
    isError = false,
    metadata: unknown | null = null,
  ): Block | null {
    const index = findPendingToolCallIndex(this.blocks, toolUseId)
    if (index === null) return null
    const block = this.blocks[index] as Extract<Block, { type: 'tool_call' }>

    block.status = isError ? 'error' : 'completed'
    block.output = output
    block.streamingOutput = []
    block.metadata = metadata
    this.recordBlockReplace(index)
    return block
  }

  findPendingToolUseId(): string | null {
    return this.pendingToolCalls()[0]?.toolUseId ?? null
  }

  pendingToolCalls(): Extract<Block, { type: 'tool_call' }>[] {
    return this.blocks.filter((block): block is Extract<Block, { type: 'tool_call' }> => {
      return block.type === 'tool_call' && block.status === 'executing'
    })
  }

  removeDanglingToolCalls(): Block[] {
    const removed: Block[] = []
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      const block = this.blocks[i]
      if (block.type !== 'tool_call' || block.status !== 'executing') continue
      removed.unshift(...this.blocks.splice(i, 1))
      this.record({ op: 'remove', path: ['blocks', i] })
    }
    return removed
  }

  findLastCompactionIndex(): number | null {
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      if (this.blocks[i].type === 'compaction_boundary') return i
    }
    return null
  }

  findCompactionCutPoint(keepRecentTokens: number): number | null {
    return this.findCompactionWindow(keepRecentTokens)?.cutPoint ?? null
  }

  findCompactionWindow(keepRecentTokens: number): CompactionWindow | null {
    const startIndex = this.findLastCompactionIndex() ?? 0
    let recentTokens = 0
    let splitFallback: CompactionWindow | null = null

    for (let i = this.blocks.length - 1; i > startIndex; i -= 1) {
      const block = this.blocks[i]
      recentTokens += estimateBlockTokens(block)
      if (recentTokens >= keepRecentTokens) {
        if (block.type === 'response') return splitFallback ?? { startIndex, cutPoint: i + 1 }
        splitFallback ??= { startIndex, cutPoint: i }
      }
    }
    return splitFallback
  }

  insertCompactionBoundary(
    index: number,
    model: ModelSelection,
    summary: string,
    summaryTokens: number,
  ): Block {
    const block: Block = {
      type: 'compaction_boundary',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
      summary,
      summaryTokens,
    }
    this.blocks.splice(index, 0, block)
    this.recordAdd(index, block)
    return block
  }

  appendCompactionMarker(model: ModelSelection, boundaryId: string, compactedTokens: number): Block {
    return this.appendBlock({
      type: 'compaction_marker',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
      boundaryId,
      compactedTokens,
    })
  }

  appendExtensionStateSnapshot(extensionName: string, state: unknown): Block {
    return this.appendBlock({
      type: 'extension_state_snapshot',
      id: this.idFactory(),
      createdAt: this.now(),
      extensionName,
      state,
    })
  }

  latestExtensionStateSnapshot(extensionName?: string): Extract<Block, { type: 'extension_state_snapshot' }> | null {
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      const block = this.blocks[i]
      if (block.type !== 'extension_state_snapshot') continue
      if (extensionName === undefined || block.extensionName === extensionName) return block
    }
    return null
  }

  replayableBlocks(): Block[] {
    const index = this.findLastCompactionIndex()
    return index === null ? [...this.blocks] : this.blocks.slice(index)
  }

  collectInferenceItems(): InferenceItem[] {
    const items: InferenceItem[] = []
    for (const block of this.replayableBlocks()) {
      switch (block.type) {
        case 'user':
          items.push({
            type: 'user_message',
            content:
              block.preamble === null
                ? boundUserContent(block.content, this.replayHeadChars, this.replayTailChars)
                : boundUserContent(
                    [{ type: 'text', text: block.preamble }, ...block.content],
                    this.replayHeadChars,
                    this.replayTailChars,
                  ),
          })
          break
        case 'resume':
          items.push({
            type: 'user_message',
            content: [{ type: 'text', text: 'Continue from where you left off.' }],
          })
          break
        case 'steer':
          items.push({
            type: 'user_steer',
            turnId: block.turnId,
            content: boundUserContent(block.content, this.replayHeadChars, this.replayTailChars),
          })
          break
        case 'thinking':
          items.push({
            type: 'assistant_thinking',
            modelId: block.model.model.id,
            text: boundText(block.text, this.replayHeadChars, this.replayTailChars),
            signature: block.signature,
          })
          break
        case 'redacted_thinking':
          items.push({
            type: 'assistant_redacted_thinking',
            modelId: block.model.model.id,
            data: boundText(block.data, this.replayHeadChars, this.replayTailChars),
          })
          break
        case 'text':
          items.push({ type: 'assistant_text', modelId: block.model.model.id, text: boundText(block.text, this.replayHeadChars, this.replayTailChars) })
          break
        case 'tool_call':
          items.push({
            type: 'tool_use',
            modelId: block.model.model.id,
            toolUseId: block.toolUseId,
            toolName: block.toolName,
            input: parseToolInput(block.input),
          })
          if (block.status !== 'executing') {
            items.push({
              type: 'tool_result',
              toolUseId: block.toolUseId,
              output: boundToolResultContent(block.output, this.replayHeadChars, this.replayTailChars),
              isError: block.status === 'error',
            })
          }
          break
        case 'compaction_boundary':
          items.push({
            type: 'user_message',
            content: [
              {
                type: 'text',
                text: boundText(`Previous conversation summary:\n${block.summary}`, this.replayHeadChars, this.replayTailChars),
              },
            ],
          })
          break
      }
    }
    return items
  }

  /**
   * Estimates the next request's context size. When a provider-reported usage
   * exists after the last compaction (the most recent real measurement of this
   * history), it anchors the estimate and only blocks streamed after it are
   * char-estimated; otherwise the whole replay window is estimated at ~4
   * chars/token with fixed weights for images and documents.
   *
   * When `contextWindow` is given, an anchor larger than the window is
   * discarded: a single request's usage physically cannot exceed the window,
   * so such a value is a provider violation of the response-usage contract
   * (e.g. a turn-cumulative total) and would poison the estimate.
   */
  estimateContextTokens(contextWindow?: number): number {
    let anchor = this.usageAnchor()
    if (anchor !== null && contextWindow !== undefined && contextWindow > 0 && anchor.tokens > contextWindow) {
      anchor = null
    }
    if (anchor === null) {
      return this.replayableBlocks().reduce((total, block) => total + estimateBlockTokens(block), 0)
    }
    let total = anchor.tokens
    for (let i = anchor.blockIndex + 1; i < this.blocks.length; i += 1) {
      total += estimateBlockTokens(this.blocks[i])
    }
    return total
  }

  /**
   * The latest response block's usage, valid only when no compaction happened
   * after it (compaction shrinks the history the usage was measured against).
   */
  private usageAnchor(): { blockIndex: number; tokens: number } | null {
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      const block = this.blocks[i]
      if (block.type === 'compaction_boundary' || block.type === 'compaction_marker') return null
      if (block.type !== 'response') continue
      const usage = block.usage
      const tokens = usage.inputTokens + usage.outputTokens + usage.cacheReadTokens + usage.cacheWriteTokens
      if (tokens <= 0) continue
      return { blockIndex: i, tokens }
    }
    return null
  }

  private appendText(model: ModelSelection, text: string): Block {
    const previous = this.blocks[this.blocks.length - 1]
    if (previous?.type === 'text') {
      previous.text += text
      this.record({ op: 'append_text', path: ['blocks', this.blocks.length - 1], delta: text })
      return previous
    }
    return this.appendBlock({
      type: 'text',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
      text,
    })
  }

  private appendThinking(model: ModelSelection, text: string): Block {
    const previous = this.blocks[this.blocks.length - 1]
    if (previous?.type === 'thinking') {
      previous.text += text
      this.record({ op: 'append_text', path: ['blocks', this.blocks.length - 1], delta: text })
      return previous
    }
    return this.appendBlock({
      type: 'thinking',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
      text,
      signature: null,
    })
  }

  private setLatestThinkingSignature(signature: string): Block | null {
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      const block = this.blocks[i]
      if (block.type === 'thinking') {
        block.signature = signature
        this.recordBlockReplace(i)
        return block
      }
    }
    return null
  }

  private appendBlock(block: Block): Block {
    this.blocks.push(block)
    this.recordAdd(this.blocks.length - 1, block)
    return block
  }
}

function findPendingToolCallIndex(blocks: Block[], toolUseId: string): number | null {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index]
    if (block.type === 'tool_call' && block.status === 'executing' && block.toolUseId === toolUseId) return index
  }
  return null
}

function stringifyToolInput(input: unknown): string {
  return typeof input === 'string' ? input : safeStringify(input, 'null')
}

function parseToolInput(input: string): unknown {
  try {
    return JSON.parse(input)
  } catch {
    return input
  }
}

function estimateBlockTokens(block: Block): number {
  return Math.ceil(estimateBlockText(block).length / 4) + estimateBlockMediaTokens(block)
}

/** Char-based per-block estimate, exported for compaction metrics. */
export function estimateTranscriptBlockTokens(block: Block): number {
  return estimateBlockTokens(block)
}

function estimateBlockMediaTokens(block: Block): number {
  switch (block.type) {
    case 'user':
    case 'steer':
      return block.content.reduce((total, content) => total + userContentMediaTokens(content), 0)
    case 'tool_call':
      return block.output.reduce((total, content) => total + toolResultMediaTokens(content), 0)
    default:
      return 0
  }
}

function userContentMediaTokens(content: UserContentBlock): number {
  if (content.type === 'image') {
    if (content.source.type === 'binary') {
      return Math.max(IMAGE_BASE_TOKENS, Math.ceil(content.source.data.byteLength / IMAGE_BYTES_PER_TOKEN))
    }
    return IMAGE_BASE_TOKENS
  }
  if (content.type === 'document') {
    return Math.ceil(content.source.data.byteLength / DOCUMENT_BYTES_PER_TOKEN)
  }
  return 0
}

function toolResultMediaTokens(content: ToolResultContentBlock): number {
  if (content.type !== 'image') return 0
  const bytes = Math.floor((content.source.data.length * 3) / 4)
  return Math.max(IMAGE_BASE_TOKENS, Math.ceil(bytes / IMAGE_BYTES_PER_TOKEN))
}

function estimateBlockText(block: Block): string {
  switch (block.type) {
    case 'user':
      return block.content.map(stringifyUserContent).join('\n')
    case 'resume':
      return 'Continue from where you left off.'
    case 'steer':
      return block.content.map(stringifyUserContent).join('\n')
    case 'thinking':
      return block.text
    case 'redacted_thinking':
      return block.data
    case 'text':
      return block.text
    case 'tool_call':
      return `${block.toolName}\n${block.input}\n${block.output.map(stringifyToolResult).join('\n')}`
    case 'response':
      return safeStringify(block.usage, '')
    case 'error':
      return block.message
    case 'abort':
      return 'aborted'
    case 'compaction_boundary':
      return block.summary
    case 'compaction_marker':
      return String(block.compactedTokens)
    case 'extension_state_snapshot':
      return safeStringify(block.state, '')
  }
}

function safeStringify(value: unknown, fallback: string): string {
  if (typeof value === 'string') return value
  return safeJsonStringify(value) ?? fallback
}

function stringifyUserContent(content: UserContentBlock): string {
  switch (content.type) {
    case 'text':
      return content.text
    case 'image':
      return content.source.type === 'url' ? content.source.url : content.source.mediaType
    case 'video':
      return content.source.type === 'url' ? content.source.url : content.source.mediaType
    case 'document':
      return `${content.source.fileName} ${content.source.mediaType}`
    case 'reference':
      return content.reference
  }
}

function stringifyToolResult(content: ToolResultContentBlock): string {
  switch (content.type) {
    case 'text':
      return content.text
    case 'image':
      return content.source.mediaType
    case 'video':
      return content.source.mediaType
  }
}

function boundUserContent(content: UserContentBlock[], headChars: number, tailChars: number): UserContentBlock[] {
  return content.map((block) => {
    if (block.type !== 'text') return block
    const text = boundText(block.text, headChars, tailChars)
    return text === block.text ? block : { ...block, text }
  })
}

function boundToolResultContent(
  content: ToolResultContentBlock[],
  headChars: number,
  tailChars: number,
): ToolResultContentBlock[] {
  return content.map((block) => {
    if (block.type !== 'text') return block
    const text = boundText(block.text, headChars, tailChars)
    return text === block.text ? block : { ...block, text }
  })
}

function boundText(text: string, headChars: number, tailChars: number): string {
  const maxChars = headChars + tailChars
  if (text.length <= maxChars) return text
  const omitted = text.length - maxChars
  return `${text.slice(0, headChars)}\n\n[... truncated ${omitted} characters ...]\n\n${text.slice(-tailChars)}`
}
