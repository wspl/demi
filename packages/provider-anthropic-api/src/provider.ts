import { isAbortError, isRecord, normalizeBaseUrl, numberOrNull, numberOrZero, parseJsonObject, parseJsonOrString, stringOrNull } from '@demi/utils'
import { Buffer } from 'node:buffer'
import process from 'node:process'
import { zeroUsage } from '@demi/core'
import type { TokenUsage, ToolResultContentBlock, UserContentBlock } from '@demi/core'
import {
  defineProvider,
  httpErrorCode,
  normalizeErrorCode,
  providerErrorFromUnknown,
  redactSecretText,
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
  anthropicApiDefaultModels,
  modelListFromAnthropicApiModels,
  type AnthropicApiModelOptions,
} from './models'

export type AnthropicApiSecretResolver = () => string | Promise<string> | null | undefined
export type AnthropicApiHeadersResolver = () => Record<string, string> | Promise<Record<string, string>>
export type AnthropicApiFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

export interface AnthropicApiRequestOptions {
  maxTokens?: number
  extraBody?: Record<string, unknown>
}

export interface AnthropicApiProviderOptions {
  id?: string
  displayName?: string
  envPrefix?: string
  baseUrl?: string
  apiKey?: AnthropicApiSecretResolver
  headers?: AnthropicApiHeadersResolver
  anthropicVersion?: string
  models?: AnthropicApiModelOptions[]
  defaultModelId?: string
  request?: AnthropicApiRequestOptions
  fetch?: AnthropicApiFetch
}

interface AnthropicApiRuntimeOptions {
  baseUrl: string
  apiKey: AnthropicApiSecretResolver
  headers?: AnthropicApiHeadersResolver
  anthropicVersion: string
  request?: AnthropicApiRequestOptions
  fetch: AnthropicApiFetch
}

const DEFAULT_ANTHROPIC_API_BASE_URL = 'https://api.anthropic.com/v1'
const DEFAULT_ANTHROPIC_VERSION = '2023-06-01'
const DEFAULT_MAX_TOKENS = 4096

export class AnthropicApiProvider implements AgentProvider {
  constructor(private readonly options: AnthropicApiRuntimeOptions) {}

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
      if (!apiKey && !headers.has('x-api-key')) {
        yield { type: 'error', message: 'Anthropic API key is missing', code: 'auth_missing' }
        return
      }
    } catch (error) {
      yield providerErrorFromUnknown(error, apiKey)
      return
    }

    try {
      const response = await this.options.fetch(anthropicMessagesUrl(this.options.baseUrl), {
        method: 'POST',
        headers,
        body: JSON.stringify(buildAnthropicMessagesBody(request, this.options.request)),
        signal: request.cancel,
      })
      if (!response.ok) {
        yield await httpError(response, apiKey)
        return
      }
      yield* mapAnthropicMessageStream(readServerSentEvents(response.body, request.cancel), request.cancel)
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
    headers.set('anthropic-version', this.options.anthropicVersion)
    if (apiKey) headers.set('x-api-key', apiKey)
    return headers
  }
}

export function createAnthropicApiProvider(options: AnthropicApiProviderOptions = {}): Provider {
  const id = options.id ?? 'anthropic'
  const displayName = options.displayName ?? 'Anthropic API'
  const envPrefix = options.envPrefix ?? 'ANTHROPIC'
  const baseUrl = normalizeBaseUrl(options.baseUrl ?? process.env[`${envPrefix}_BASE_URL`] ?? DEFAULT_ANTHROPIC_API_BASE_URL)
  const apiKey = options.apiKey ?? (() => process.env[`${envPrefix}_API_KEY`])
  const fetchImpl = options.fetch ?? fetch
  const modelList = (): ProviderModelList =>
    options.models
      ? modelListFromAnthropicApiModels(options.models, { providerId: id, defaultModelId: options.defaultModelId ?? null })
      : withProviderId(anthropicApiDefaultModels(id), id)
  const runtimeOptions: AnthropicApiRuntimeOptions = {
    baseUrl,
    apiKey,
    headers: options.headers,
    anthropicVersion: options.anthropicVersion ?? DEFAULT_ANTHROPIC_VERSION,
    request: options.request,
    fetch: fetchImpl,
  }

  return defineProvider({
    id,
    displayName,
    auth: { status: () => authStatus(apiKey, options.headers, 'x-api-key') },
    state: () => ({ status: 'ready', message: 'Uses the Anthropic Messages API' }),
    listModels: modelList,
    createRuntime: (_selection: ProviderSelection) => new AnthropicApiProvider(runtimeOptions),
  })
}

export interface AnthropicMessagesRequestBody {
  model: string
  messages: AnthropicMessage[]
  max_tokens: number
  stream: true
  system?: string
  tools?: AnthropicTool[]
  thinking?: { type: 'enabled'; budget_tokens: number }
  service_tier?: string
  [key: string]: unknown
}

export interface AnthropicMessage {
  role: 'user' | 'assistant'
  content: AnthropicContentBlock[]
}

export type AnthropicContentBlock =
  | { type: 'text'; text: string }
  | { type: 'image'; source: { type: 'base64'; media_type: string; data: string } }
  | { type: 'tool_use'; id: string; name: string; input: unknown }
  | { type: 'tool_result'; tool_use_id: string; content: AnthropicToolResultContent[]; is_error?: boolean }

export type AnthropicToolResultContent =
  | { type: 'text'; text: string }
  | { type: 'image'; source: { type: 'base64'; media_type: string; data: string } }

export interface AnthropicTool {
  name: string
  description: string
  input_schema: Record<string, unknown>
}

export function buildAnthropicMessagesBody(
  request: InferenceRequest,
  options: AnthropicApiRequestOptions | undefined,
): AnthropicMessagesRequestBody {
  const body: AnthropicMessagesRequestBody = {
    model: request.modelId,
    messages: inferenceItemsToAnthropicMessages(request.items),
    max_tokens: options?.maxTokens ?? DEFAULT_MAX_TOKENS,
    stream: true,
  }
  if (request.systemPrompt.trim()) body.system = request.systemPrompt
  if (request.tools.length > 0) body.tools = request.tools.map(toolToAnthropicTool)
  if (request.thinking?.type === 'budget') {
    body.thinking = { type: 'enabled', budget_tokens: request.thinking.budgetTokens }
  }
  if (request.serviceTierId) body.service_tier = request.serviceTierId
  if (options?.extraBody) Object.assign(body, options.extraBody)
  return body
}

export async function* mapAnthropicMessageStream(
  events: AsyncIterable<ServerSentEvent>,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const toolBlocks = new Map<number, AnthropicToolBlock>()
  let usage = zeroUsage()

  for await (const event of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    for (const data of event.data) {
      const value = parseJsonObject(data)
      if (!value) continue
      const type = stringOrNull(value.type) ?? event.event

      if (type === 'error') {
        const error = isRecord(value.error) ? value.error : value
        const message = stringOrNull(error.message) ?? 'Anthropic API stream error'
        yield { type: 'error', message, code: normalizeErrorCode(stringOrNull(error.type), message) }
        return
      }

      if (type === 'message_start') {
        const message = isRecord(value.message) ? value.message : null
        const messageUsage = message && isRecord(message.usage) ? message.usage : null
        if (messageUsage) usage = mergeAnthropicUsage(usage, messageUsage)
        continue
      }

      if (type === 'content_block_start') {
        const index = numberOrNull(value.index) ?? 0
        const block = isRecord(value.content_block) ? value.content_block : null
        if (block?.type === 'tool_use') {
          toolBlocks.set(index, {
            id: stringOrNull(block.id) ?? `tool_use_${index}`,
            name: stringOrNull(block.name) ?? '',
            initialInput: block.input,
            inputJson: '',
          })
        } else if (block?.type === 'thinking') {
          yield { type: 'thinking_start' }
        } else if (block?.type === 'text') {
          const text = stringOrNull(block.text)
          if (text) yield { type: 'text_delta', text }
        }
        continue
      }

      if (type === 'content_block_delta') {
        const index = numberOrNull(value.index) ?? 0
        const delta = isRecord(value.delta) ? value.delta : null
        if (!delta) continue
        if (delta.type === 'text_delta') {
          const text = stringOrNull(delta.text)
          if (text) yield { type: 'text_delta', text }
        } else if (delta.type === 'thinking_delta') {
          const text = stringOrNull(delta.thinking)
          if (text) yield { type: 'thinking_delta', text }
        } else if (delta.type === 'signature_delta') {
          const signature = stringOrNull(delta.signature)
          if (signature) yield { type: 'thinking_signature', signature }
        } else if (delta.type === 'input_json_delta') {
          const block = toolBlocks.get(index)
          if (block) block.inputJson += stringOrNull(delta.partial_json) ?? ''
        }
        continue
      }

      if (type === 'content_block_stop') {
        const index = numberOrNull(value.index) ?? 0
        const block = toolBlocks.get(index)
        if (block && block.name) {
          yield {
            type: 'tool_call_requested',
            toolUseId: block.id,
            toolName: block.name,
            input: block.inputJson ? parseJsonOrString(block.inputJson) : block.initialInput ?? {},
          }
        }
        toolBlocks.delete(index)
        continue
      }

      if (type === 'message_delta') {
        const deltaUsage = isRecord(value.usage) ? value.usage : null
        if (deltaUsage) usage = mergeAnthropicUsage(usage, deltaUsage)
        continue
      }

      if (type === 'message_stop') {
        yield { type: 'response', usage }
        return
      }
    }
  }

  yield { type: 'response', usage }
}

interface AnthropicToolBlock {
  id: string
  name: string
  initialInput: unknown
  inputJson: string
}

function inferenceItemsToAnthropicMessages(items: InferenceItem[]): AnthropicMessage[] {
  const messages: AnthropicMessage[] = []

  const append = (role: AnthropicMessage['role'], content: AnthropicContentBlock[]) => {
    const last = messages[messages.length - 1]
    if (last?.role === role) {
      last.content.push(...content)
      return
    }
    messages.push({ role, content })
  }

  for (const item of items) {
    switch (item.type) {
      case 'user_message':
      case 'user_steer':
        append('user', userContentToAnthropic(item.content))
        break
      case 'assistant_text':
        append('assistant', [{ type: 'text', text: item.text }])
        break
      case 'tool_use':
        append('assistant', [{ type: 'tool_use', id: item.toolUseId, name: item.toolName, input: item.input ?? {} }])
        break
      case 'tool_result':
        append('user', [{
          type: 'tool_result',
          tool_use_id: item.toolUseId,
          content: toolResultContentToAnthropic(item.output),
          ...(item.isError ? { is_error: true } : {}),
        }])
        break
      case 'assistant_thinking':
      case 'assistant_redacted_thinking':
        break
    }
  }

  return messages
}

function userContentToAnthropic(content: UserContentBlock[]): AnthropicContentBlock[] {
  return content.flatMap((block): AnthropicContentBlock[] => {
    if (block.type === 'text') return [{ type: 'text', text: block.text }]
    if (block.type === 'reference') return [{ type: 'text', text: block.reference }]
    if (block.type === 'document') return [{ type: 'text', text: `[document:${block.source.fileName} ${block.source.mediaType}]` }]
    if (block.source.type === 'url') return [{ type: 'text', text: `[image:${block.source.url}]` }]
    return [{
      type: 'image',
      source: {
        type: 'base64',
        media_type: block.source.mediaType,
        data: Buffer.from(block.source.data).toString('base64'),
      },
    }]
  })
}

function toolResultContentToAnthropic(output: ToolResultContentBlock[]): AnthropicToolResultContent[] {
  return output.map((block) => {
    if (block.type === 'text') return { type: 'text', text: block.text }
    return {
      type: 'image',
      source: { type: 'base64', media_type: block.source.mediaType, data: block.source.data },
    }
  })
}

function toolToAnthropicTool(tool: ToolDefinition): AnthropicTool {
  return {
    name: tool.name,
    description: tool.description,
    input_schema: tool.inputSchema,
  }
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

function anthropicMessagesUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/messages') ? normalized : `${normalized}/messages`
}

async function authStatus(apiKey: AnthropicApiSecretResolver, headersResolver: AnthropicApiHeadersResolver | undefined, authHeader: string) {
  const [key, headers] = await Promise.all([apiKey(), headersResolver?.()])
  if (key || (headers && Object.keys(headers).some((name) => name.toLowerCase() === authHeader))) {
    return { status: 'authenticated' as const }
  }
  return { status: 'unauthenticated' as const, message: 'Anthropic API key is missing' }
}

async function httpError(response: Response, apiKey: string | null | undefined): Promise<ProviderEvent> {
  const text = await response.text().catch(() => '')
  const message = redactSecretText(`Anthropic API request failed with HTTP ${response.status}${text ? `: ${text}` : ''}`, apiKey)
  return { type: 'error', message, code: httpErrorCode(response.status, message) }
}

function mergeAnthropicUsage(current: TokenUsage, usage: Record<string, unknown>): TokenUsage {
  const inputTokens = numberOrZero(usage.input_tokens)
  const outputTokens = numberOrZero(usage.output_tokens)
  const cacheReadTokens = numberOrZero(usage.cache_read_input_tokens)
  const cacheWriteTokens = numberOrZero(usage.cache_creation_input_tokens)
  return {
    inputTokens: inputTokens || current.inputTokens,
    outputTokens: outputTokens || current.outputTokens,
    cacheReadTokens: cacheReadTokens || current.cacheReadTokens,
    cacheWriteTokens: cacheWriteTokens || current.cacheWriteTokens,
  }
}

function withProviderId(list: ProviderModelList, providerId: string): ProviderModelList {
  return {
    ...list,
    providerId,
    models: list.models.map((model) => ({ ...model, providerId })),
  }
}

