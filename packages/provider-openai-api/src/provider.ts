import { Buffer } from 'node:buffer'
import process from 'node:process'
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

export interface OpenAIApiProviderOptions {
  id?: string
  displayName?: string
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

export class OpenAIApiProvider implements AgentProvider {
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

export function createOpenAIApiProvider(options: OpenAIApiProviderOptions = {}): Provider {
  const id = options.id ?? 'openai'
  const displayName = options.displayName ?? 'OpenAI API'
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
    state: () => ({ status: 'ready', message: 'Uses the OpenAI Chat Completions API' }),
    listModels: modelList,
    createRuntime: (_selection: ProviderSelection) => new OpenAIApiProvider(runtimeOptions),
  })
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
          message: stringOr(error.message) ?? 'OpenAI API stream error',
          code: normalizeErrorCode(stringOr(error.code) ?? stringOr(error.type), stringOr(error.message) ?? ''),
        }
        return
      }
      if (isRecord(chunk.usage)) usage = openAIUsage(chunk.usage)
      const choices = Array.isArray(chunk.choices) ? chunk.choices : []
      for (const choice of choices) {
        if (!isRecord(choice)) continue
        const delta = isRecord(choice.delta) ? choice.delta : null
        if (delta) {
          const content = stringOr(delta.content)
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
    const id = stringOr(value.id)
    if (id) existing.id = id
    const name = stringOr(fn?.name)
    if (name) existing.name = name
    const delta = stringOr(fn?.arguments)
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

function normalizeBaseUrl(baseUrl: string): string {
  return baseUrl.replace(/\/+$/, '')
}

function thinkingToReasoningEffort(request: InferenceRequest): string | undefined {
  const thinking = request.thinking
  if (!thinking || thinking.type === 'disabled' || thinking.type === 'budget') return undefined
  return thinking.effort
}

function stringifyToolArguments(input: unknown): string {
  return typeof input === 'string' ? input : JSON.stringify(input ?? {})
}

function parseJsonOrString(value: string): unknown {
  try {
    return JSON.parse(value)
  } catch {
    return value
  }
}

function parseJsonObject(value: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(value)
    return isRecord(parsed) ? parsed : null
  } catch {
    return null
  }
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

function zeroUsage() {
  return { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }
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

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError'
}

function withProviderId(list: ProviderModelList, providerId: string): ProviderModelList {
  return {
    ...list,
    providerId,
    models: list.models.map((model) => ({ ...model, providerId })),
  }
}

function stringOr(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function numberOrZero(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}
