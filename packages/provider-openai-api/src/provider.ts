import { isAbortError, isRecord, normalizeBaseUrl, numberOrZero, parseJsonObject, parseJsonOrString, shortHash, stringOrNull } from '@demi/utils'
import { Buffer } from 'node:buffer'
import process from 'node:process'
import { zeroUsage } from '@demi/core'
import type { ToolResultContentBlock, UserContentBlock } from '@demi/core'
import {
  defineProvider,
  type AgentProvider,
  type InferenceItem,
  type InferenceRequest,
  type Provider,
  type ProviderEvent,
  type ProviderModelList,
  type ProviderSelection,
  type ToolDefinition,
} from '@demi/provider'
import {
  modelListFromOpenAIApiModels,
  openAIApiDefaultModels,
  type OpenAIApiModelOptions,
} from './models'

export type OpenAIApiSecretResolver = () => string | Promise<string> | null | undefined
export type OpenAIApiHeadersResolver = () => Record<string, string> | Promise<Record<string, string>>
export type OpenAIApiFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

export interface OpenAIApiRequestOptions {
  maxRetries?: number
  streamOptions?: Record<string, unknown> | null
  extraBody?: Record<string, unknown>
}

export type OpenAIApiWireApi = 'responses' | 'chat-completions'

export interface OpenAIApiProviderOptions {
  id?: string
  displayName?: string
  wireApi?: OpenAIApiWireApi
  envPrefix?: string
  baseUrl?: string
  apiKey?: OpenAIApiSecretResolver
  headers?: OpenAIApiHeadersResolver
  models?: OpenAIApiModelOptions[]
  defaultModelId?: string
  request?: OpenAIApiRequestOptions
  fetch?: OpenAIApiFetch
}

interface OpenAIApiRuntimeOptions {
  baseUrl: string
  apiKey: OpenAIApiSecretResolver
  headers?: OpenAIApiHeadersResolver
  request?: OpenAIApiRequestOptions
  fetch: OpenAIApiFetch
}

const DEFAULT_OPENAI_API_BASE_URL = 'https://api.openai.com/v1'

export class OpenAIChatCompletionsProvider implements AgentProvider {
  constructor(private readonly options: OpenAIApiRuntimeOptions) {}

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (request.cancel.aborted) {
      yield { type: 'abort' }
      return
    }

    let apiKey: string | null | undefined
    let headers: Headers
    try {
      apiKey = await this.options.apiKey()
      headers = await this.buildHeaders(apiKey)
      if (!apiKey && !headers.has('authorization')) {
        yield { type: 'error', message: 'OpenAI API key is missing', code: 'auth_missing' }
        return
      }
    } catch (error) {
      yield providerErrorFromUnknown(error, apiKey)
      return
    }

    try {
      const response = await this.options.fetch(openAIChatCompletionsUrl(this.options.baseUrl), {
        method: 'POST',
        headers,
        body: JSON.stringify(buildOpenAIChatCompletionsBody(request, this.options.request)),
        signal: request.cancel,
      })
      if (!response.ok) {
        yield await httpError(response, apiKey)
        return
      }
      yield* mapOpenAIChatCompletionStream(readServerSentEvents(response.body, request.cancel), request.cancel)
    } catch (error) {
      if (request.cancel.aborted || isAbortError(error)) {
        yield { type: 'abort' }
        return
      }
      yield providerErrorFromUnknown(error, apiKey)
    }
  }

  private async buildHeaders(apiKey: string | null | undefined): Promise<Headers> {
    const headers = new Headers(await this.options.headers?.())
    headers.set('accept', 'text/event-stream')
    headers.set('content-type', 'application/json')
    if (apiKey) headers.set('authorization', `Bearer ${apiKey}`)
    return headers
  }
}

export class OpenAIResponsesProvider implements AgentProvider {
  constructor(private readonly options: OpenAIApiRuntimeOptions) {}

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (request.cancel.aborted) {
      yield { type: 'abort' }
      return
    }

    let apiKey: string | null | undefined
    let headers: Headers
    try {
      apiKey = await this.options.apiKey()
      headers = await this.buildHeaders(apiKey)
      if (!apiKey && !headers.has('authorization')) {
        yield { type: 'error', message: 'OpenAI API key is missing', code: 'auth_missing' }
        return
      }
    } catch (error) {
      yield providerErrorFromUnknown(error, apiKey)
      return
    }

    try {
      const response = await this.options.fetch(openAIResponsesUrl(this.options.baseUrl), {
        method: 'POST',
        headers,
        body: JSON.stringify(buildOpenAIResponsesBody(request, this.options.request)),
        signal: request.cancel,
      })
      if (!response.ok) {
        yield await httpError(response, apiKey)
        return
      }
      yield* mapOpenAIResponseStream(readServerSentEvents(response.body, request.cancel), request.cancel)
    } catch (error) {
      if (request.cancel.aborted || isAbortError(error)) {
        yield { type: 'abort' }
        return
      }
      yield providerErrorFromUnknown(error, apiKey)
    }
  }

  private async buildHeaders(apiKey: string | null | undefined): Promise<Headers> {
    const headers = new Headers(await this.options.headers?.())
    headers.set('accept', 'text/event-stream')
    headers.set('content-type', 'application/json')
    if (apiKey) headers.set('authorization', `Bearer ${apiKey}`)
    return headers
  }
}

export function createOpenAIApiProvider(options: OpenAIApiProviderOptions = {}): Provider {
  const id = options.id ?? 'openai'
  const displayName = options.displayName ?? 'OpenAI API'
  const wireApi = options.wireApi ?? 'responses'
  const envPrefix = options.envPrefix ?? 'OPENAI'
  const baseUrl = normalizeBaseUrl(options.baseUrl ?? process.env[`${envPrefix}_BASE_URL`] ?? DEFAULT_OPENAI_API_BASE_URL)
  const apiKey = options.apiKey ?? (() => process.env[`${envPrefix}_API_KEY`])
  const fetchImpl = options.fetch ?? fetch
  const modelList = (): ProviderModelList =>
    options.models
      ? modelListFromOpenAIApiModels(options.models, { providerId: id, defaultModelId: options.defaultModelId ?? null })
      : withProviderId(openAIApiDefaultModels(id), id)
  const runtimeOptions: OpenAIApiRuntimeOptions = {
    baseUrl,
    apiKey,
    headers: options.headers,
    request: options.request,
    fetch: fetchImpl,
  }

  return defineProvider({
    id,
    displayName,
    auth: { status: () => authStatus(apiKey, options.headers, 'authorization') },
    state: () => ({
      status: 'ready',
      message: wireApi === 'responses' ? 'Uses the OpenAI Responses API' : 'Uses the OpenAI Chat Completions API',
    }),
    listModels: modelList,
    createRuntime: (_selection: ProviderSelection) =>
      wireApi === 'responses' ? new OpenAIResponsesProvider(runtimeOptions) : new OpenAIChatCompletionsProvider(runtimeOptions),
  })
}

export interface OpenAIResponsesRequestBody {
  model: string
  input: OpenAIResponseInputItem[]
  stream: true
  instructions?: string
  tools?: OpenAIResponseTool[]
  tool_choice?: 'auto'
  parallel_tool_calls?: boolean
  reasoning?: { effort?: string; summary?: string }
  service_tier?: string
  store: false
  include?: string[]
  prompt_cache_key?: string
  stream_options?: Record<string, unknown>
  [key: string]: unknown
}

export type OpenAIResponseInputItem =
  | { type: 'message'; role: 'assistant'; content: Array<{ type: 'output_text'; text: string; annotations: unknown[] }>; id?: string; status?: 'completed' }
  | { role: 'user'; content: OpenAIResponseUserContent[] }
  | { type: 'reasoning'; id?: string; summary?: Array<{ type?: string; text: string }>; content?: Array<{ type?: string; text: string }>; encrypted_content?: string }
  | { type: 'function_call'; id?: string; call_id: string; name: string; arguments: string }
  | { type: 'function_call_output'; call_id: string; output: string }

export type OpenAIResponseUserContent =
  | { type: 'input_text'; text: string }
  | { type: 'input_image'; image_url: string; detail?: 'auto' | 'low' | 'high' }

export interface OpenAIResponseTool {
  type: 'function'
  name: string
  description: string
  parameters: Record<string, unknown>
}

export interface OpenAIResponseStreamEvent {
  type?: string
  response?: OpenAIResponseCompleted | OpenAIResponseFailed
  item?: OpenAIResponseOutputItem
  delta?: string
  arguments?: string
  item_id?: string
  call_id?: string
  code?: string
  message?: string
  [key: string]: unknown
}

export type OpenAIResponseOutputItem =
  | OpenAIResponseReasoningItem
  | OpenAIResponseMessageItem
  | OpenAIResponseFunctionCallItem
  | Record<string, unknown>

export interface OpenAIResponseReasoningItem {
  type: 'reasoning'
  id?: string
  summary?: Array<{ type?: string; text: string }>
  content?: Array<{ type?: string; text: string }>
  encrypted_content?: string
}

export interface OpenAIResponseMessageItem {
  type: 'message'
  id?: string
  role?: string
  content?: Array<{ type: 'output_text'; text: string } | { type: 'refusal'; refusal: string }>
  status?: string
}

export interface OpenAIResponseFunctionCallItem {
  type: 'function_call'
  id?: string
  call_id?: string
  name?: string
  arguments?: string
}

export interface OpenAIResponseCompleted {
  id?: string
  status?: string
  usage?: {
    input_tokens?: number
    output_tokens?: number
    total_tokens?: number
    input_tokens_details?: { cached_tokens?: number }
  }
  [key: string]: unknown
}

export interface OpenAIResponseFailed {
  error?: { code?: string; type?: string; message?: string }
  incomplete_details?: { reason?: string }
  [key: string]: unknown
}

interface OpenAIResponseStreamState {
  currentReasoning: OpenAIResponseReasoningItem | null
  currentFunctionCall: OpenAIResponseFunctionCallItem | null
  functionArguments: Map<string, string>
  reasoningDeltaSeen: boolean
}

export function buildOpenAIResponsesBody(
  request: InferenceRequest,
  options: OpenAIApiRequestOptions | undefined,
): OpenAIResponsesRequestBody {
  const body: OpenAIResponsesRequestBody = {
    model: request.modelId,
    input: request.items.flatMap((item, index) => inferenceItemToOpenAIResponseInput(item, index)),
    stream: true,
    store: false,
    include: ['reasoning.encrypted_content'],
    prompt_cache_key: clampPromptCacheKey(request.sessionId),
  }
  if (request.systemPrompt.trim()) body.instructions = request.systemPrompt
  if (request.tools.length > 0) {
    body.tools = request.tools.map(toolToOpenAIResponseTool)
    body.tool_choice = 'auto'
    body.parallel_tool_calls = true
  }
  const reasoning = thinkingToOpenAIReasoning(request.thinking)
  if (reasoning) body.reasoning = reasoning
  if (request.serviceTierId) body.service_tier = request.serviceTierId
  if (options?.streamOptions !== undefined && options.streamOptions !== null) body.stream_options = options.streamOptions
  if (options?.extraBody) Object.assign(body, options.extraBody)
  return body
}

export async function* mapOpenAIResponseStream(
  events: AsyncIterable<ServerSentEvent>,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const state = newOpenAIResponseStreamState()
  let completed = false

  for await (const event of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    for (const data of event.data) {
      if (data === '[DONE]') continue
      const parsed = parseJsonObject(data)
      if (!parsed) continue
      const streamEvent = parsed as OpenAIResponseStreamEvent
      if (streamEvent.type === 'response.completed') completed = true
      yield* mapOpenAIResponseEvent(streamEvent, state)
    }
  }

  if (!completed) yield { type: 'response', usage: zeroUsage() }
}

export function* mapOpenAIResponseEvent(
  event: OpenAIResponseStreamEvent,
  state: OpenAIResponseStreamState = newOpenAIResponseStreamState(),
): Iterable<ProviderEvent> {
  switch (event.type) {
    case 'response.output_item.added': {
      const item = event.item
      if (isOpenAIResponseReasoningItem(item)) {
        state.currentReasoning = item
        yield { type: 'thinking_start' }
      } else if (isOpenAIResponseFunctionCallItem(item)) {
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
      if (isOpenAIResponseReasoningItem(item)) {
        if (!state.reasoningDeltaSeen) {
          const text = openAIResponseReasoningText(item)
          if (text) yield { type: 'thinking_delta', text }
        }
        yield { type: 'thinking_signature', signature: JSON.stringify(item) }
        if (state.currentReasoning === item) state.currentReasoning = null
        state.reasoningDeltaSeen = false
      } else if (isOpenAIResponseMessageItem(item)) {
        const text = openAIResponseMessageText(item)
        if (text) yield { type: 'text_delta', text }
      } else if (isOpenAIResponseFunctionCallItem(item)) {
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
      yield { type: 'response', usage: openAIResponsesUsage(event.response) }
      return
    case 'response.failed':
      yield openAIResponseErrorEvent(event.response)
      return
    case 'response.incomplete': {
      const reason = openAIIncompleteReason(event.response)
      yield {
        type: 'error',
        message: `Incomplete OpenAI response returned, reason: ${reason}`,
        code: reason === 'max_output_tokens' ? 'context_length_exceeded' : 'incomplete',
      }
      return
    }
    case 'error':
      yield { type: 'error', message: event.message ?? 'OpenAI API stream error', code: event.code ?? null }
      return
  }
}

export interface OpenAIChatCompletionsRequestBody {
  model: string
  messages: OpenAIChatMessage[]
  stream: true
  tools?: OpenAIChatTool[]
  tool_choice?: 'auto'
  service_tier?: string
  reasoning_effort?: string
  stream_options?: Record<string, unknown>
  [key: string]: unknown
}

export type OpenAIChatMessage =
  | { role: 'system'; content: string }
  | { role: 'user'; content: string | OpenAIUserContentPart[] }
  | { role: 'assistant'; content: string | null; tool_calls?: OpenAIChatToolCall[] }
  | { role: 'tool'; tool_call_id: string; content: string }

export type OpenAIUserContentPart =
  | { type: 'text'; text: string }
  | { type: 'image_url'; image_url: { url: string; detail?: 'auto' | 'low' | 'high' } }

export interface OpenAIChatTool {
  type: 'function'
  function: {
    name: string
    description: string
    parameters: Record<string, unknown>
  }
}

export interface OpenAIChatToolCall {
  id: string
  type: 'function'
  function: {
    name: string
    arguments: string
  }
}

export function buildOpenAIChatCompletionsBody(
  request: InferenceRequest,
  options: OpenAIApiRequestOptions | undefined,
): OpenAIChatCompletionsRequestBody {
  const body: OpenAIChatCompletionsRequestBody = {
    model: request.modelId,
    messages: inferenceItemsToOpenAIMessages(request.systemPrompt, request.items),
    stream: true,
  }
  if (request.tools.length > 0) {
    body.tools = request.tools.map(toolToOpenAITool)
    body.tool_choice = 'auto'
  }
  const reasoningEffort = thinkingToReasoningEffort(request)
  if (reasoningEffort) body.reasoning_effort = reasoningEffort
  if (request.serviceTierId) body.service_tier = request.serviceTierId
  if (options?.streamOptions !== null) body.stream_options = options?.streamOptions ?? { include_usage: true }
  if (options?.extraBody) Object.assign(body, options.extraBody)
  return body
}

export async function* mapOpenAIChatCompletionStream(
  events: AsyncIterable<ServerSentEvent>,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const toolCalls = new Map<number, MutableOpenAIToolCall>()
  let thinkingStarted = false
  let usage = zeroUsage()

  for await (const event of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    for (const data of event.data) {
      if (data === '[DONE]') {
        yield* flushOpenAIToolCalls(toolCalls)
        yield { type: 'response', usage }
        return
      }
      const chunk = parseJsonObject(data)
      if (!chunk) continue
      const error = isRecord(chunk.error) ? chunk.error : null
      if (error) {
        yield {
          type: 'error',
          message: stringOrNull(error.message) ?? 'OpenAI API stream error',
          code: normalizeErrorCode(stringOrNull(error.code) ?? stringOrNull(error.type), stringOrNull(error.message) ?? ''),
        }
        return
      }
      if (isRecord(chunk.usage)) usage = openAIUsage(chunk.usage)
      const choices = Array.isArray(chunk.choices) ? chunk.choices : []
      for (const choice of choices) {
        if (!isRecord(choice)) continue
        const delta = isRecord(choice.delta) ? choice.delta : null
        if (delta) {
          const reasoning = stringOrNull(delta.reasoning_content)
          if (reasoning) {
            if (!thinkingStarted) {
              thinkingStarted = true
              yield { type: 'thinking_start' }
            }
            yield { type: 'thinking_delta', text: reasoning }
          }
          const content = stringOrNull(delta.content)
          if (content) yield { type: 'text_delta', text: content }
          if (Array.isArray(delta.tool_calls)) collectOpenAIToolCalls(delta.tool_calls, toolCalls)
        }
        if (choice.finish_reason === 'tool_calls') yield* flushOpenAIToolCalls(toolCalls)
      }
    }
  }

  yield* flushOpenAIToolCalls(toolCalls)
  yield { type: 'response', usage }
}

interface MutableOpenAIToolCall {
  id: string
  name: string
  arguments: string
}

function inferenceItemsToOpenAIMessages(systemPrompt: string, items: InferenceItem[]): OpenAIChatMessage[] {
  const messages: OpenAIChatMessage[] = []
  let assistant: { content: string; toolCalls: OpenAIChatToolCall[] } | null = null

  const flushAssistant = () => {
    if (!assistant) return
    messages.push({
      role: 'assistant',
      content: assistant.content || null,
      ...(assistant.toolCalls.length > 0 ? { tool_calls: assistant.toolCalls } : {}),
    })
    assistant = null
  }

  if (systemPrompt.trim()) messages.push({ role: 'system', content: systemPrompt })

  for (const item of items) {
    switch (item.type) {
      case 'user_message':
      case 'user_steer':
        flushAssistant()
        messages.push({ role: 'user', content: userContentToOpenAI(item.content) })
        break
      case 'assistant_text':
        assistant ??= { content: '', toolCalls: [] }
        assistant.content += item.text
        break
      case 'tool_use':
        assistant ??= { content: '', toolCalls: [] }
        assistant.toolCalls.push({
          id: item.toolUseId,
          type: 'function',
          function: { name: item.toolName, arguments: stringifyToolArguments(item.input) },
        })
        break
      case 'tool_result':
        flushAssistant()
        messages.push({ role: 'tool', tool_call_id: item.toolUseId, content: toolResultContentToText(item.output) })
        break
      case 'assistant_thinking':
      case 'assistant_redacted_thinking':
        break
    }
  }
  flushAssistant()
  return messages
}

function userContentToOpenAI(content: UserContentBlock[]): string | OpenAIUserContentPart[] {
  const parts: OpenAIUserContentPart[] = []
  for (const block of content) {
    if (block.type === 'text') parts.push({ type: 'text', text: block.text })
    else if (block.type === 'reference') parts.push({ type: 'text', text: block.reference })
    else if (block.type === 'document') {
      parts.push({ type: 'text', text: `[document:${block.source.fileName} ${block.source.mediaType}]` })
    } else if (block.source.type === 'url') {
      parts.push({ type: 'image_url', image_url: { url: block.source.url, detail: 'auto' } })
    } else {
      parts.push({
        type: 'image_url',
        image_url: {
          url: `data:${block.source.mediaType};base64,${Buffer.from(block.source.data).toString('base64')}`,
          detail: 'auto',
        },
      })
    }
  }
  if (parts.every((part) => part.type === 'text')) return parts.map((part) => part.text).join('\n')
  return parts
}

function toolResultContentToText(output: ToolResultContentBlock[]): string {
  return output.map((block) => (block.type === 'text' ? block.text : `[image:${block.source.mediaType}]`)).join('\n')
}

function toolToOpenAITool(tool: ToolDefinition): OpenAIChatTool {
  return {
    type: 'function',
    function: {
      name: tool.name,
      description: tool.description,
      parameters: tool.inputSchema,
    },
  }
}

function collectOpenAIToolCalls(values: unknown[], toolCalls: Map<number, MutableOpenAIToolCall>): void {
  for (const value of values) {
    if (!isRecord(value)) continue
    const index = typeof value.index === 'number' ? value.index : toolCalls.size
    const existing = toolCalls.get(index) ?? { id: '', name: '', arguments: '' }
    const fn = isRecord(value.function) ? value.function : null
    const id = stringOrNull(value.id)
    if (id) existing.id = id
    const name = stringOrNull(fn?.name)
    if (name) existing.name = name
    const delta = stringOrNull(fn?.arguments)
    if (delta) existing.arguments += delta
    toolCalls.set(index, existing)
  }
}

function* flushOpenAIToolCalls(toolCalls: Map<number, MutableOpenAIToolCall>): Iterable<ProviderEvent> {
  for (const [index, call] of [...toolCalls.entries()].sort(([a], [b]) => a - b)) {
    if (!call.name) continue
    yield {
      type: 'tool_call_requested',
      toolUseId: call.id || `tool_call_${index}`,
      toolName: call.name,
      input: parseJsonOrString(call.arguments || '{}'),
    }
  }
  toolCalls.clear()
}

function inferenceItemToOpenAIResponseInput(item: InferenceItem, index: number): OpenAIResponseInputItem[] {
  switch (item.type) {
    case 'user_message':
    case 'user_steer':
      return [{ role: 'user', content: userContentToOpenAIResponses(item.content) }]
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
      const reasoning = parseOpenAIReasoningSignature(item.signature)
      return reasoning ? [reasoning] : []
    }
    case 'assistant_redacted_thinking':
      return []
    case 'tool_use': {
      const { callId, itemId } = splitOpenAIResponseToolUseId(item.toolUseId)
      return [
        {
          type: 'function_call',
          id: itemId,
          call_id: callId,
          name: item.toolName,
          arguments: stringifyToolArguments(item.input),
        },
      ]
    }
    case 'tool_result': {
      const { callId } = splitOpenAIResponseToolUseId(item.toolUseId)
      return [{ type: 'function_call_output', call_id: callId, output: toolResultContentToText(item.output) }]
    }
  }
}

function userContentToOpenAIResponses(content: UserContentBlock[]): OpenAIResponseUserContent[] {
  return content.flatMap((block): OpenAIResponseUserContent[] => {
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

function toolToOpenAIResponseTool(tool: ToolDefinition): OpenAIResponseTool {
  return {
    type: 'function',
    name: tool.name,
    description: tool.description,
    parameters: tool.inputSchema,
  }
}

function thinkingToOpenAIReasoning(thinking: InferenceRequest['thinking']): { effort?: string; summary?: string } | undefined {
  if (!thinking || thinking.type === 'disabled' || thinking.type === 'budget') return undefined
  if (thinking.type === 'adaptive') return { effort: thinking.effort, summary: 'auto' }
  if (thinking.effort === 'none') return { effort: 'none' }
  return { effort: thinking.effort, summary: thinking.summary && thinking.summary !== 'off' ? thinking.summary : 'auto' }
}

function splitOpenAIResponseToolUseId(toolUseId: string): { callId: string; itemId: string | undefined } {
  const [callId, itemId] = toolUseId.split('|', 2)
  return { callId, itemId }
}

function parseOpenAIReasoningSignature(signature: string | null): OpenAIResponseInputItem | null {
  if (!signature) return null
  try {
    const parsed = JSON.parse(signature)
    return isOpenAIResponseReasoningItem(parsed) ? parsed : null
  } catch {
    return null
  }
}

function openAIResponsesUsage(response: unknown) {
  const usage = isRecord(response) && isRecord(response.usage) ? response.usage : {}
  const inputTokens = numberOrZero(usage.input_tokens)
  const inputDetails = isRecord(usage.input_tokens_details) ? usage.input_tokens_details : null
  const cachedTokens = inputDetails ? numberOrZero(inputDetails.cached_tokens) : 0
  return {
    inputTokens: Math.max(0, inputTokens - cachedTokens),
    outputTokens: numberOrZero(usage.output_tokens),
    cacheReadTokens: cachedTokens,
    cacheWriteTokens: 0,
  }
}

function openAIResponseErrorEvent(response: unknown): ProviderEvent {
  const error = isRecord(response) && isRecord(response.error) ? response.error : null
  const message = stringOrNull(error?.message) ?? 'OpenAI response failed'
  const rawCode = stringOrNull(error?.code) ?? stringOrNull(error?.type)
  return { type: 'error', message, code: normalizeErrorCode(rawCode, message) }
}

function openAIIncompleteReason(response: unknown): string {
  if (isRecord(response) && isRecord(response.incomplete_details) && typeof response.incomplete_details.reason === 'string') {
    return response.incomplete_details.reason
  }
  return 'unknown'
}

function openAIResponseMessageText(item: OpenAIResponseMessageItem): string {
  return (
    item.content
      ?.map((part) => (part.type === 'output_text' ? part.text : part.type === 'refusal' ? part.refusal : ''))
      .join('') ?? ''
  )
}

function openAIResponseReasoningText(item: OpenAIResponseReasoningItem): string {
  const summary = item.summary?.map((part) => part.text).join('\n\n') ?? ''
  const content = item.content?.map((part) => part.text).join('\n\n') ?? ''
  return summary || content
}

function newOpenAIResponseStreamState(): OpenAIResponseStreamState {
  return {
    currentReasoning: null,
    currentFunctionCall: null,
    functionArguments: new Map(),
    reasoningDeltaSeen: false,
  }
}

function isOpenAIResponseReasoningItem(item: unknown): item is OpenAIResponseReasoningItem {
  return isRecord(item) && item.type === 'reasoning'
}

function isOpenAIResponseMessageItem(item: unknown): item is OpenAIResponseMessageItem {
  return isRecord(item) && item.type === 'message'
}

function isOpenAIResponseFunctionCallItem(item: unknown): item is OpenAIResponseFunctionCallItem {
  return isRecord(item) && item.type === 'function_call'
}

export interface ServerSentEvent {
  event: string | null
  data: string[]
}

export async function* readServerSentEvents(
  body: ReadableStream<Uint8Array> | null,
  signal?: AbortSignal,
): AsyncIterable<ServerSentEvent> {
  if (!body) return
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  let eventName: string | null = null
  let data: string[] = []

  const flush = function* (): Iterable<ServerSentEvent> {
    if (data.length === 0) return
    yield { event: eventName, data }
    eventName = null
    data = []
  }

  try {
    while (true) {
      if (signal?.aborted) return
      const { value, done } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      let newline = buffer.indexOf('\n')
      while (newline !== -1) {
        const raw = buffer.slice(0, newline)
        buffer = buffer.slice(newline + 1)
        const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw
        if (line === '') {
          yield* flush()
        } else if (line.startsWith('event:')) {
          eventName = line.slice('event:'.length).trim()
        } else if (line.startsWith('data:')) {
          data.push(line.slice('data:'.length).trimStart())
        }
        newline = buffer.indexOf('\n')
      }
    }
    buffer += decoder.decode()
    if (buffer) {
      const line = buffer.endsWith('\r') ? buffer.slice(0, -1) : buffer
      if (line.startsWith('data:')) data.push(line.slice('data:'.length).trimStart())
      else if (line.startsWith('event:')) eventName = line.slice('event:'.length).trim()
    }
    yield* flush()
  } finally {
    reader.releaseLock()
  }
}

function openAIChatCompletionsUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/chat/completions') ? normalized : `${normalized}/chat/completions`
}

function openAIResponsesUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/responses') ? normalized : `${normalized}/responses`
}

function thinkingToReasoningEffort(request: InferenceRequest): string | undefined {
  const thinking = request.thinking
  if (!thinking || thinking.type === 'disabled' || thinking.type === 'budget') return undefined
  return thinking.effort
}

function stringifyToolArguments(input: unknown): string {
  return typeof input === 'string' ? input : JSON.stringify(input ?? {})
}

function clampPromptCacheKey(value: string): string {
  return value.length <= 64 ? value : `session_${shortHash(value)}`
}

function openAIUsage(usage: Record<string, unknown>) {
  const inputTokens = numberOrZero(usage.prompt_tokens)
  const outputTokens = numberOrZero(usage.completion_tokens)
  const promptDetails = isRecord(usage.prompt_tokens_details) ? usage.prompt_tokens_details : null
  const cachedTokens = promptDetails ? numberOrZero(promptDetails.cached_tokens) : 0
  return {
    inputTokens: Math.max(0, inputTokens - cachedTokens),
    outputTokens,
    cacheReadTokens: cachedTokens,
    cacheWriteTokens: 0,
  }
}

async function authStatus(apiKey: OpenAIApiSecretResolver, headersResolver: OpenAIApiHeadersResolver | undefined, authHeader: string) {
  const [key, headers] = await Promise.all([apiKey(), headersResolver?.()])
  if (key || (headers && Object.keys(headers).some((name) => name.toLowerCase() === authHeader))) {
    return { status: 'authenticated' as const }
  }
  return { status: 'unauthenticated' as const, message: 'OpenAI API key is missing' }
}

async function httpError(response: Response, apiKey: string | null | undefined): Promise<ProviderEvent> {
  const text = await response.text().catch(() => '')
  const message = redactSecretText(`OpenAI API request failed with HTTP ${response.status}${text ? `: ${text}` : ''}`, apiKey)
  return { type: 'error', message, code: httpErrorCode(response.status, message) }
}

function providerErrorFromUnknown(error: unknown, apiKey: string | null | undefined): ProviderEvent {
  const message = error instanceof Error ? error.message : String(error)
  return { type: 'error', message: redactSecretText(message, apiKey), code: normalizeErrorCode(null, message) }
}

function httpErrorCode(status: number, message: string): string | null {
  if (status === 401 || status === 403) return 'auth_expired'
  if (status === 408 || status === 409 || status === 425 || status === 429 || status >= 500) return 'rate_limit'
  if (status === 400 && /context|too long|token/i.test(message)) return 'context_length_exceeded'
  return null
}

function normalizeErrorCode(code: string | null, message: string): string | null {
  const value = `${code ?? ''} ${message}`.toLowerCase()
  if (/context|too long|max.*token/.test(value)) return 'context_length_exceeded'
  if (/rate|quota|billing|limit/.test(value)) return 'rate_limit'
  if (/auth|unauth|invalid.*key|expired/.test(value)) return 'auth_expired'
  if (/overload|unavailable|timeout/.test(value)) return 'overloaded'
  return code
}

function redactSecretText(value: string, secret: string | null | undefined): string {
  return secret ? value.split(secret).join('[redacted]') : value
}

function withProviderId(list: ProviderModelList, providerId: string): ProviderModelList {
  return {
    ...list,
    providerId,
    models: list.models.map((model) => ({ ...model, providerId })),
  }
}

