import type {
  Block,
  ModelSelection,
  ToolResultContentBlock,
  Transcript as CoreTranscript,
  UserContentBlock,
} from '@demi/core'
import type { InferenceItem, ProviderEvent } from '@demi/provider'

const MODEL_TEXT_HEAD_CHARS = 8_000
const MODEL_TEXT_TAIL_CHARS = 8_000
const MODEL_TEXT_MAX_CHARS = MODEL_TEXT_HEAD_CHARS + MODEL_TEXT_TAIL_CHARS

export interface TranscriptOptions {
  idFactory?: () => string
  now?: () => string
}

export interface CompactionWindow {
  startIndex: number
  cutPoint: number
}

export class Transcript implements CoreTranscript {
  readonly blocks: Block[]

  private readonly idFactory: () => string
  private readonly now: () => string

  constructor(blocks: Block[] = [], options: TranscriptOptions = {}) {
    this.blocks = [...blocks]
    this.idFactory = options.idFactory ?? defaultIdFactory
    this.now = options.now ?? (() => new Date().toISOString())
  }

  snapshot(): CoreTranscript {
    return { blocks: [...this.blocks] }
  }

  pushUserTurn(model: ModelSelection, content: UserContentBlock[], preamble: string | null = null): Block {
    const block: Block = {
      type: 'user',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
      content,
      preamble,
    }
    this.blocks.push(block)
    return block
  }

  pushResumeTurn(model: ModelSelection): Block {
    const block: Block = {
      type: 'resume',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
    }
    this.blocks.push(block)
    return block
  }

  pushAbort(model: ModelSelection, isResumed = false): Block {
    const block: Block = {
      type: 'abort',
      id: this.idFactory(),
      createdAt: this.now(),
      model,
      isResumed,
    }
    this.blocks.push(block)
    return block
  }

  markLatestAbortResumed(): boolean {
    for (let i = this.blocks.length - 1; i >= 0; i -= 1) {
      const block = this.blocks[i]
      if (block.type === 'abort') {
        block.isResumed = true
        return true
      }
    }
    return false
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
    const block = this.blocks.find((candidate) => {
      return candidate.type === 'tool_call' && candidate.toolUseId === toolUseId
    })
    if (block?.type !== 'tool_call') return null

    block.status = isError ? 'error' : 'completed'
    block.output = output
    block.streamingOutput = []
    block.metadata = metadata
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
                ? boundUserContent(block.content)
                : boundUserContent([{ type: 'text', text: block.preamble }, ...block.content]),
          })
          break
        case 'resume':
          items.push({
            type: 'user_message',
            content: [{ type: 'text', text: 'Continue from where you left off.' }],
          })
          break
        case 'thinking':
          items.push({
            type: 'assistant_thinking',
            modelId: block.model.model.id,
            text: boundText(block.text),
            signature: block.signature,
          })
          break
        case 'redacted_thinking':
          items.push({
            type: 'assistant_redacted_thinking',
            modelId: block.model.model.id,
            data: boundText(block.data),
          })
          break
        case 'text':
          items.push({ type: 'assistant_text', modelId: block.model.model.id, text: boundText(block.text) })
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
              output: boundToolResultContent(block.output),
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
                text: boundText(`Previous conversation summary:\n${block.summary}`),
              },
            ],
          })
          break
      }
    }
    return items
  }

  estimateContextTokens(): number {
    return this.replayableBlocks().reduce((total, block) => total + estimateBlockTokens(block), 0)
  }

  private appendText(model: ModelSelection, text: string): Block {
    const previous = this.blocks[this.blocks.length - 1]
    if (previous?.type === 'text') {
      previous.text += text
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
        return block
      }
    }
    return null
  }

  private appendBlock(block: Block): Block {
    this.blocks.push(block)
    return block
  }
}

function defaultIdFactory(): string {
  return globalThis.crypto.randomUUID()
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
  return Math.ceil(estimateBlockText(block).length / 4)
}

function estimateBlockText(block: Block): string {
  switch (block.type) {
    case 'user':
      return block.content.map(stringifyUserContent).join('\n')
    case 'resume':
      return 'Continue from where you left off.'
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
  const seen = new WeakSet<object>()
  try {
    const text = JSON.stringify(value, (_key, nested) => {
      if (typeof nested === 'bigint') return nested.toString()
      if (typeof nested === 'symbol') return String(nested)
      if (typeof nested === 'function') return `[Function ${nested.name || 'anonymous'}]`
      if (nested !== null && typeof nested === 'object') {
        if (seen.has(nested)) return '[Circular]'
        seen.add(nested)
      }
      return nested
    })
    return text ?? fallback
  } catch {
    return fallback
  }
}

function stringifyUserContent(content: UserContentBlock): string {
  switch (content.type) {
    case 'text':
      return content.text
    case 'image':
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
  }
}

function boundUserContent(content: UserContentBlock[]): UserContentBlock[] {
  return content.map((block) => {
    if (block.type !== 'text') return block
    const text = boundText(block.text)
    return text === block.text ? block : { ...block, text }
  })
}

function boundToolResultContent(content: ToolResultContentBlock[]): ToolResultContentBlock[] {
  return content.map((block) => {
    if (block.type !== 'text') return block
    const text = boundText(block.text)
    return text === block.text ? block : { ...block, text }
  })
}

function boundText(text: string): string {
  if (text.length <= MODEL_TEXT_MAX_CHARS) return text
  const omitted = text.length - MODEL_TEXT_MAX_CHARS
  return `${text.slice(0, MODEL_TEXT_HEAD_CHARS)}\n\n[... truncated ${omitted} characters ...]\n\n${text.slice(-MODEL_TEXT_TAIL_CHARS)}`
}
