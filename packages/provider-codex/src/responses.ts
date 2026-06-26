import { isRecord } from '@demi/utils'
import { Buffer } from 'node:buffer'
import type { TokenUsage, ToolResultContentBlock, UserContentBlock } from '@demi/core'
import type { InferenceItem, InferenceRequest, ProviderEvent, ToolDefinition } from '@demi/provider'

export interface CodexResponsesRequestBody {
  model: string
  instructions: string
  input: CodexResponseInputItem[]
  tools: CodexResponseTool[]
  tool_choice: 'auto'
  parallel_tool_calls: boolean
  reasoning?: { effort?: string; summary?: string }
  service_tier?: string
  store: boolean
  stream: boolean
  include: string[]
  prompt_cache_key: string
  text?: { verbosity?: 'low' | 'medium' | 'high' }
  [key: string]: unknown
}

export type CodexResponseInputItem =
  | { type: 'message'; role: 'assistant'; content: Array<{ type: 'output_text'; text: string; annotations: unknown[] }>; id?: string; status?: 'completed'; phase?: string }
  | { role: 'user'; content: CodexUserContent[] }
  | { type: 'reasoning'; id?: string; summary?: Array<{ type?: string; text: string }>; content?: Array<{ type?: string; text: string }>; encrypted_content?: string }
  | { type: 'function_call'; id?: string; call_id: string; name: string; arguments: string }
  | { type: 'function_call_output'; call_id: string; output: string | CodexUserContent[] }

export type CodexUserContent =
  | { type: 'input_text'; text: string }
  | { type: 'input_image'; image_url: string; detail?: 'auto' | 'low' | 'high' }

export interface CodexResponseTool {
  type: 'function'
  name: string
  description: string
  parameters: Record<string, unknown>
  strict: null
}

export interface CodexResponseStreamEvent {
  type?: string
  response?: CodexResponseCompleted | CodexResponseFailed
  item?: CodexResponseOutputItem
  delta?: string
  arguments?: string
  item_id?: string
  call_id?: string
  summary_index?: number
  content_index?: number
  code?: string
  message?: string
  [key: string]: unknown
}

export type CodexResponseOutputItem =
  | CodexReasoningItem
  | CodexMessageItem
  | CodexFunctionCallItem
  | Record<string, unknown>

export interface CodexReasoningItem {
  type: 'reasoning'
  id?: string
  summary?: Array<{ type?: string; text: string }>
  content?: Array<{ type?: string; text: string }>
  encrypted_content?: string
}

export interface CodexMessageItem {
  type: 'message'
  id?: string
  role?: string
  content?: Array<{ type: 'output_text'; text: string } | { type: 'refusal'; refusal: string }>
  status?: string
  phase?: string
}

export interface CodexFunctionCallItem {
  type: 'function_call'
  id?: string
  call_id?: string
  name?: string
  arguments?: string
}

export interface CodexResponseCompleted {
  id?: string
  status?: string
  end_turn?: boolean
  usage?: {
    input_tokens?: number
    output_tokens?: number
    total_tokens?: number
    input_tokens_details?: { cached_tokens?: number }
  }
  [key: string]: unknown
}

export interface CodexResponseFailed {
  error?: { code?: string; type?: string; message?: string }
  incomplete_details?: { reason?: string }
  [key: string]: unknown
}

interface StreamState {
  currentReasoning: CodexReasoningItem | null
  currentFunctionCall: CodexFunctionCallItem | null
  functionArguments: Map<string, string>
  reasoningDeltaSeen: boolean
}

export function buildCodexResponsesRequestBody(request: InferenceRequest): CodexResponsesRequestBody {
  const body: CodexResponsesRequestBody = {
    model: request.modelId,
    instructions: request.systemPrompt,
    input: request.items.flatMap((item, index) => inferenceItemToResponsesInput(item, index)),
    tools: request.tools.map(toolToResponsesTool),
    tool_choice: 'auto',
    parallel_tool_calls: true,
    store: false,
    stream: true,
    include: ['reasoning.encrypted_content'],
    prompt_cache_key: clampPromptCacheKey(request.sessionId),
    text: { verbosity: 'low' },
  }
  const reasoning = thinkingToReasoning(request.thinking)
  if (reasoning) body.reasoning = reasoning
  if (request.serviceTierId) body.service_tier = request.serviceTierId
  return body
}

export async function* mapCodexResponseEvents(events: AsyncIterable<CodexResponseStreamEvent>): AsyncIterable<ProviderEvent> {
  const state: StreamState = {
    currentReasoning: null,
    currentFunctionCall: null,
    functionArguments: new Map(),
    reasoningDeltaSeen: false,
  }

  for await (const event of events) {
    yield* mapCodexResponseEvent(event, state)
  }
}

export function* mapCodexResponseEvent(event: CodexResponseStreamEvent, state: StreamState = newStreamState()): Iterable<ProviderEvent> {
  switch (event.type) {
    case 'response.output_item.added': {
      const item = event.item
      if (isReasoningItem(item)) {
        state.currentReasoning = item
        yield { type: 'thinking_start' }
      } else if (isFunctionCallItem(item)) {
        state.currentFunctionCall = item
        if (item.id) state.functionArguments.set(item.id, item.arguments ?? '')
      }
      return
    }
    case 'response.reasoning_summary_text.delta':
    case 'response.reasoning_text.delta':
      if (typeof event.delta === 'string') {
        state.reasoningDeltaSeen = true
        yield { type: 'thinking_delta', text: event.delta }
      }
      return
    case 'response.output_text.delta':
      if (typeof event.delta === 'string') yield { type: 'text_delta', text: event.delta }
      return
    case 'response.function_call_arguments.delta': {
      const key = event.item_id ?? state.currentFunctionCall?.id
      if (key && typeof event.delta === 'string') {
        state.functionArguments.set(key, `${state.functionArguments.get(key) ?? ''}${event.delta}`)
      }
      return
    }
    case 'response.function_call_arguments.done': {
      const key = event.item_id ?? state.currentFunctionCall?.id
      if (key && typeof event.arguments === 'string') state.functionArguments.set(key, event.arguments)
      return
    }
    case 'response.output_item.done': {
      const item = event.item
      if (isReasoningItem(item)) {
        if (!state.reasoningDeltaSeen) {
          const text = reasoningText(item)
          if (text) yield { type: 'thinking_delta', text }
        }
        yield { type: 'thinking_signature', signature: JSON.stringify(item) }
        if (state.currentReasoning === item) state.currentReasoning = null
        state.reasoningDeltaSeen = false
      } else if (isMessageItem(item)) {
        const text = messageText(item)
        if (text) yield { type: 'text_delta', text }
      } else if (isFunctionCallItem(item)) {
        const itemId = item.id ?? event.item_id
        const callId = item.call_id ?? event.call_id
        if (itemId && callId && item.name) {
          const rawArgs = state.functionArguments.get(itemId) ?? item.arguments ?? '{}'
          yield {
            type: 'tool_call_requested',
            toolUseId: `${callId}|${itemId}`,
            toolName: item.name,
            input: parseJsonOrString(rawArgs),
          }
        }
        if (itemId) state.functionArguments.delete(itemId)
        if (state.currentFunctionCall === item) state.currentFunctionCall = null
      }
      return
    }
    case 'response.completed':
      yield { type: 'response', usage: usageFromResponse(event.response) }
      return
    case 'response.failed':
      yield errorEventFromFailedResponse(event.response)
      return
    case 'response.incomplete':
      yield {
        type: 'error',
        message: `Incomplete response returned, reason: ${incompleteReason(event.response)}`,
        code: incompleteReason(event.response) === 'max_output_tokens' ? 'context_length_exceeded' : 'incomplete',
      }
      return
    case 'error':
      yield { type: 'error', message: event.message ?? 'Codex stream error', code: event.code ?? null }
      return
  }
}

export function splitCodexToolUseId(toolUseId: string): { callId: string; itemId: string | undefined } {
  const [callId, itemId] = toolUseId.split('|', 2)
  return { callId, itemId }
}

export function usageFromResponse(response: unknown): TokenUsage {
  const usage = isRecord(response) && isRecord(response.usage) ? response.usage : {}
  const inputTokens = numberOrZero(usage.input_tokens)
  const cachedTokens = isRecord(usage.input_tokens_details) ? numberOrZero(usage.input_tokens_details.cached_tokens) : 0
  return {
    inputTokens: Math.max(0, inputTokens - cachedTokens),
    outputTokens: numberOrZero(usage.output_tokens),
    cacheReadTokens: cachedTokens,
    cacheWriteTokens: 0,
  }
}

function inferenceItemToResponsesInput(item: InferenceItem, index: number): CodexResponseInputItem[] {
  switch (item.type) {
    case 'user_message':
    case 'user_steer':
      return [{ role: 'user', content: userContentToResponses(item.content) }]
    case 'assistant_text':
      return [
        {
          type: 'message',
          role: 'assistant',
          content: [{ type: 'output_text', text: item.text, annotations: [] }],
          id: `msg_${shortHash(`${index}:${item.modelId}:${item.text}`)}`,
          status: 'completed',
        },
      ]
    case 'assistant_thinking': {
      const reasoning = parseReasoningSignature(item.signature)
      return reasoning ? [reasoning] : []
    }
    case 'assistant_redacted_thinking':
      return []
    case 'tool_use': {
      const { callId, itemId } = splitCodexToolUseId(item.toolUseId)
      return [
        {
          type: 'function_call',
          id: itemId,
          call_id: callId,
          name: item.toolName,
          arguments: stringifyArguments(item.input),
        },
      ]
    }
    case 'tool_result': {
      const { callId } = splitCodexToolUseId(item.toolUseId)
      return [{ type: 'function_call_output', call_id: callId, output: toolResultToResponsesOutput(item.output) }]
    }
  }
}

function userContentToResponses(content: UserContentBlock[]): CodexUserContent[] {
  return content.flatMap((block): CodexUserContent[] => {
    if (block.type === 'text') return [{ type: 'input_text', text: block.text }]
    if (block.type === 'reference') return [{ type: 'input_text', text: block.reference }]
    if (block.type === 'document') {
      return [{ type: 'input_text', text: `[document:${block.source.fileName} ${block.source.mediaType}]` }]
    }
    if (block.source.type === 'url') return [{ type: 'input_image', image_url: block.source.url, detail: 'auto' }]
    const base64 = Buffer.from(block.source.data.buffer, block.source.data.byteOffset, block.source.data.byteLength).toString('base64')
    return [{ type: 'input_image', image_url: `data:${block.source.mediaType};base64,${base64}`, detail: 'auto' }]
  })
}

function toolResultToResponsesOutput(output: ToolResultContentBlock[]): string | CodexUserContent[] {
  const images = output.filter((block) => block.type === 'image')
  const text = output.filter((block) => block.type === 'text').map((block) => block.text).join('\n')
  if (images.length === 0) return text
  const content: CodexUserContent[] = []
  if (text) content.push({ type: 'input_text', text })
  for (const image of images) {
    content.push({ type: 'input_image', image_url: `data:${image.source.mediaType};base64,${image.source.data}`, detail: 'auto' })
  }
  return content
}

function toolToResponsesTool(tool: ToolDefinition): CodexResponseTool {
  return {
    type: 'function',
    name: tool.name,
    description: tool.description,
    parameters: tool.inputSchema,
    strict: null,
  }
}

function thinkingToReasoning(thinking: InferenceRequest['thinking']): { effort?: string; summary?: string } | undefined {
  if (!thinking || thinking.type === 'disabled' || thinking.type === 'budget') return undefined
  if (thinking.type === 'adaptive') return { effort: thinking.effort, summary: 'auto' }
  if (thinking.effort === 'none') return { effort: 'none' }
  return { effort: thinking.effort, summary: thinking.summary && thinking.summary !== 'off' ? thinking.summary : 'auto' }
}

function parseReasoningSignature(signature: string | null): CodexResponseInputItem | null {
  if (!signature) return null
  try {
    const parsed = JSON.parse(signature)
    return isReasoningItem(parsed) ? parsed : null
  } catch {
    return null
  }
}

function stringifyArguments(input: unknown): string {
  return typeof input === 'string' ? input : JSON.stringify(input ?? {})
}

function parseJsonOrString(value: string): unknown {
  try {
    return JSON.parse(value)
  } catch {
    return value
  }
}

function messageText(item: CodexMessageItem): string {
  return (
    item.content
      ?.map((part) => (part.type === 'output_text' ? part.text : part.type === 'refusal' ? part.refusal : ''))
      .join('') ?? ''
  )
}

function reasoningText(item: CodexReasoningItem): string {
  const summary = item.summary?.map((part) => part.text).join('\n\n') ?? ''
  const content = item.content?.map((part) => part.text).join('\n\n') ?? ''
  return summary || content
}

function errorEventFromFailedResponse(response: unknown): ProviderEvent {
  const error = isRecord(response) && isRecord(response.error) ? response.error : null
  const message = stringOrNull(error?.message) ?? 'Codex response failed'
  const rawCode = stringOrNull(error?.code) ?? stringOrNull(error?.type)
  return {
    type: 'error',
    message,
    code: normalizeErrorCode(rawCode, message),
  }
}

function normalizeErrorCode(code: string | null, message: string): string | null {
  const value = `${code ?? ''} ${message}`.toLowerCase()
  if (/context|maximum context|too long|max.*token/.test(value)) return 'context_length_exceeded'
  if (/rate|quota|usage|limit|billing|balance/.test(value)) return 'rate_limit'
  if (/unauth|auth|expired|invalid.*token/.test(value)) return 'auth_expired'
  if (/overload|unavailable|timeout/.test(value)) return 'overloaded'
  return code
}

function incompleteReason(response: unknown): string {
  if (isRecord(response) && isRecord(response.incomplete_details) && typeof response.incomplete_details.reason === 'string') {
    return response.incomplete_details.reason
  }
  return 'unknown'
}

function clampPromptCacheKey(value: string): string {
  return value.length <= 64 ? value : `session_${shortHash(value)}`
}

function shortHash(value: string): string {
  let hash = 2166136261
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i)
    hash = Math.imul(hash, 16777619)
  }
  return (hash >>> 0).toString(16)
}

function newStreamState(): StreamState {
  return {
    currentReasoning: null,
    currentFunctionCall: null,
    functionArguments: new Map(),
    reasoningDeltaSeen: false,
  }
}

function isReasoningItem(item: unknown): item is CodexReasoningItem {
  return isRecord(item) && item.type === 'reasoning'
}

function isMessageItem(item: unknown): item is CodexMessageItem {
  return isRecord(item) && item.type === 'message'
}

function isFunctionCallItem(item: unknown): item is CodexFunctionCallItem {
  return isRecord(item) && item.type === 'function_call'
}

function numberOrZero(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function stringOrNull(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}
