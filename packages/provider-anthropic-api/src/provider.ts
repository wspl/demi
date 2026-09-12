import { readServerSentEvents, ProviderDataError, type ServerSentEvent } from '@demicodes/provider'
import { parseAnthropicEvent, type AnthropicUsage, type AnthropicContentBlock } from './response-schemas'
import { attachmentTag } from '@demicodes/core'
import {
  isAbortError,
  normalizeBaseUrl,
  parseJsonOrString,
} from '@demicodes/utils'
import { Buffer } from 'node:buffer'
import process from 'node:process'
import { zeroUsage } from '@demicodes/core'
import type {
  TokenUsage,
  ToolResultContentBlock,
  UserContentBlock
} from '@demicodes/core'
import {
  authStatusFromKey,
  defineProvider,
  modelListFromConfiguredModels,
  httpRequestFailedEvent,
  normalizeErrorCode,
  providerErrorFromUnknown,
  type AgentProvider,
  type InferenceItem,
  type InferenceRequest,
  type Provider,
  type ProviderEvent,
  type ProviderModelList,
  type ToolDefinition,
} from '@demicodes/provider'
import {
  anthropicApiDefaultModels,
  type AnthropicApiModelOptions,
} from './models'

export type AnthropicApiSecretResolver = () => string
  | Promise<string>
  | null
  | undefined
export type AnthropicApiHeadersResolver = () => Record<string, string>
  | Promise<Record<string, string>>
export type AnthropicApiFetch = (
  input: string | URL | Request,
  init?: RequestInit
) => Promise<Response>

export interface AnthropicApiRequestOptions {
  /**
   * Overrides the derived max_tokens (default: the model's outputLimit, else
   * 32000).
   */
  maxTokens?: number
  /**
   * Budget tokens used when an effort/adaptive thinking config is mapped onto
   * the Messages API's budget knob. Merged over the built-in ladder
   * (low 4k / medium 16k / high 32k / xhigh 64k / max 96k).
   */
  effortBudgetTokens?: Record<string, number>
  extraBody?: Record<string, unknown>
}

export interface AnthropicApiProviderOptions {
  id?: string
  displayName?: string
  envPrefix?: string
  /**
   * Messages API base, including the version prefix (usually `/v1`).
   * Only `/messages` is appended; values that already end with `/messages` are
   * used as-is. Not interchangeable with Claude Code roots that omit `/v1`.
   */
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

export const DEFAULT_ANTHROPIC_API_BASE_URL = 'https://api.anthropic.com/v1'
const DEFAULT_ANTHROPIC_VERSION = '2023-06-01'
// Agent turns routinely stream long tool-heavy responses; 4k-class defaults
// truncate them. Used only when the catalog has no outputLimit for the model.
const DEFAULT_MAX_TOKENS = 32_000
const MIN_THINKING_BUDGET_TOKENS = 1_024
const DEFAULT_EFFORT_BUDGET_TOKENS: Record<string, number> = {
  low: 4_096,
  medium: 16_384,
  high: 32_768,
  xhigh: 65_536,
  max: 98_304,
}

export class AnthropicApiProvider implements AgentProvider {
  constructor(private readonly options: AnthropicApiRuntimeOptions) {}

  clone(): AgentProvider {
    return new AnthropicApiProvider(this.options)
  }

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
        yield {
          type: 'error',
          message: 'Anthropic API key is missing',
          code: 'auth_missing'
        }
        return
      }
    } catch (error) {
      yield providerErrorFromUnknown(error, apiKey)
      return
    }

    try {
      const response = await this.options.fetch(
        anthropicMessagesUrl(this.options.baseUrl),
        {
          method: 'POST',
          headers,
          body: JSON.stringify(buildAnthropicMessagesBody(
            request,
            this.options.request
          )),
          signal: request.cancel,
        }
      )
      if (!response.ok) {
        yield await httpRequestFailedEvent(response, apiKey, 'Anthropic')
        return
      }
      yield* mapAnthropicMessageStream(
        readServerSentEvents(response.body, request.cancel),
        request.cancel
      )
    } catch (error) {
      if (request.cancel.aborted || isAbortError(error)) {
        yield { type: 'abort' }
        return
      }
      yield providerErrorFromUnknown(error, apiKey)
    }
  }

  private async buildHeaders(
    apiKey: string | null | undefined
  ): Promise<Headers> {
    const headers = new Headers(await this.options.headers?.())
    headers.set('accept', 'text/event-stream')
    headers.set('content-type', 'application/json')
    headers.set('anthropic-version', this.options.anthropicVersion)
    if (apiKey)
      headers.set('x-api-key', apiKey)
    return headers
  }
}

export function createAnthropicApiProvider(
  options: AnthropicApiProviderOptions = {}
): Provider {
  const id = options.id ?? 'anthropic'
  const displayName = options.displayName ?? 'Anthropic API'
  const envPrefix = options.envPrefix ?? 'ANTHROPIC'
  const baseUrl = normalizeBaseUrl(options.baseUrl
    ?? process.env[`${envPrefix}_BASE_URL`]
    ?? DEFAULT_ANTHROPIC_API_BASE_URL)
  const apiKey = options.apiKey ?? (() => process.env[`${envPrefix}_API_KEY`])
  const fetchImpl = options.fetch ?? fetch
  const modelList = (): ProviderModelList =>
    options.models
      ? modelListFromConfiguredModels(
        options.models,
        { providerId: id, defaultModelId: options.defaultModelId }
      )
      : anthropicApiDefaultModels(id)
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
    auth: {
      status: () => authStatusFromKey(
        apiKey,
        options.headers,
        'x-api-key',
        'Anthropic'
      )
    },
    state: () => ({
      status: 'ready',
      message: 'Uses the Anthropic Messages API'
    }),
    listModels: modelList,
    supportsOutputLimit: true,
    createRuntime: () => new AnthropicApiProvider(runtimeOptions),
  })
}

export interface AnthropicMessagesRequestBody {
  model: string
  messages: AnthropicMessage[]
  max_tokens: number
  stream: true
  system?: string
  tools?: AnthropicTool[]
  thinking?: {
    type: 'enabled';
    budget_tokens: number
  }
  service_tier?: string
  [key: string]: unknown
}

export interface AnthropicMessage {
  role: 'user' | 'assistant'
  content: AnthropicContentBlock[]
}

export type AnthropicContentBlock =
  | {
      type: 'text';
      text: string
    }
  | {
      type: 'image';
      source: {
        type: 'base64';
        media_type: string;
        data: string
      }
    }
  | {
      type: 'document';
      source: {
        type: 'base64';
        media_type: string;
        data: string
      };
      title?: string
    }
  | {
      type: 'tool_use';
      id: string;
      name: string;
      input: unknown
    }
  | {
      type: 'tool_result';
      tool_use_id: string;
      content: AnthropicToolResultContent[];
      is_error?: boolean
    }

export type AnthropicToolResultContent =
  | {
      type: 'text';
      text: string
    }
  | {
      type: 'image';
      source: {
        type: 'base64';
        media_type: string;
        data: string
      }
    }

export interface AnthropicTool {
  name: string
  description: string
  input_schema: Record<string, unknown>
}

export function buildAnthropicMessagesBody(
  request: InferenceRequest,
  options: AnthropicApiRequestOptions | undefined,
): AnthropicMessagesRequestBody {
  const maxTokens = options?.maxTokens ?? request.outputLimit
    ?? DEFAULT_MAX_TOKENS
  const body: AnthropicMessagesRequestBody = {
    model: request.modelId,
    messages: inferenceItemsToAnthropicMessages(request.items),
    max_tokens: maxTokens,
    stream: true,
  }
  if (request.systemPrompt.trim())
    body.system = request.systemPrompt
  if (request.tools.length > 0)
    body.tools = request.tools.map(toolToAnthropicTool)
  const budgetTokens = thinkingBudgetTokens(
    request.thinking,
    maxTokens,
    options?.effortBudgetTokens
  )
  if (budgetTokens !== null) {
    body.thinking = { type: 'enabled', budget_tokens: budgetTokens }
  }
  if (request.serviceTierId)
    body.service_tier = request.serviceTierId
  if (options?.extraBody)
    Object.assign(body, options.extraBody)
  return body
}

/**
 * Maps any thinking config onto the Messages API budget knob: budget configs
 * pass through, effort/adaptive configs resolve through the effort ladder
 * (caller-overridable). Budgets clamp below max_tokens and to the API minimum;
 * nothing is silently dropped.
 */
export function thinkingBudgetTokens(
  thinking: InferenceRequest['thinking'],
  maxTokens: number,
  effortBudgets: Record<string, number> | undefined,
): number | null {
  if (!thinking || thinking.type === 'disabled')
    return null
  const requested =
    thinking.type === 'budget'
      ? thinking.budgetTokens
      : (effortBudgets?.[thinking.effort] ??
        DEFAULT_EFFORT_BUDGET_TOKENS[thinking.effort] ??
        DEFAULT_EFFORT_BUDGET_TOKENS.medium!)
  const cap = Math.max(
    MIN_THINKING_BUDGET_TOKENS,
    maxTokens - MIN_THINKING_BUDGET_TOKENS
  )
  return Math.max(MIN_THINKING_BUDGET_TOKENS, Math.min(requested, cap))
}

export async function* mapAnthropicMessageStream(
  events: AsyncIterable<ServerSentEvent>,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const blocks = new Map<number, AnthropicBlockState>()
  let usage = zeroUsage()

  for await (const frame of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    const event = parseAnthropicEvent(frame)
    switch (event.type) {
      case 'error':
        yield {
          type: 'error',
          message: event.error.message,
          code: normalizeErrorCode(event.error.type, event.error.message),
        }
        return
      case 'message_start':
        if (event.message.usage)
          usage = mergeAnthropicUsage(usage, event.message.usage)
        break
      case 'content_block_start': {
        if (blocks.has(event.index))
          throw new ProviderDataError('Anthropic stream', 'duplicate active block index')
        const block = event.content_block
        if (block.type === 'tool_use') {
          blocks.set(event.index, {
            type: 'tool_use',
            id: block.id,
            name: block.name,
            initialInput: block.input,
            inputJson: '',
          })
        } else {
          blocks.set(event.index, { type: block.type })
          if (block.type === 'thinking') {
            yield { type: 'thinking_start' }
            if (block.thinking)
              yield { type: 'thinking_delta', text: block.thinking }
            if (block.signature)
              yield { type: 'thinking_signature', signature: block.signature }
          } else if (block.type === 'text' && block.text) {
            yield { type: 'text_delta', text: block.text }
          } else if (block.type === 'redacted_thinking') {
            yield { type: 'redacted_thinking', data: block.data }
          }
        }
        break
      }
      case 'content_block_delta': {
        const block = blocks.get(event.index)
        if (!block)
          throw new ProviderDataError('Anthropic stream', 'delta references an inactive block')
        const delta = event.delta
        if (delta.type === 'ignored' || block.type === 'ignored')
          break
        if (delta.type === 'input_json_delta' && block.type === 'tool_use') {
          block.inputJson += delta.partial_json
        } else if (delta.type === 'text_delta' && block.type === 'text') {
          yield { type: 'text_delta', text: delta.text }
        } else if (delta.type === 'thinking_delta' && block.type === 'thinking') {
          yield { type: 'thinking_delta', text: delta.thinking }
        } else if (delta.type === 'signature_delta' && block.type === 'thinking') {
          yield { type: 'thinking_signature', signature: delta.signature }
        } else {
          throw new ProviderDataError('Anthropic stream', 'delta type does not match its block')
        }
        break
      }
      case 'content_block_stop': {
        const block = blocks.get(event.index)
        if (!block)
          throw new ProviderDataError('Anthropic stream', 'stop references an inactive block')
        blocks.delete(event.index)
        if (block.type === 'tool_use') {
          yield {
            type: 'tool_call_requested',
            toolUseId: block.id,
            toolName: block.name,
            input: block.inputJson ? parseJsonOrString(block.inputJson) : block.initialInput,
          }
        }
        break
      }
      case 'message_delta':
        if (event.usage)
          usage = mergeAnthropicUsage(usage, event.usage)
        break
      case 'message_stop':
        if (blocks.size > 0)
          throw new ProviderDataError('Anthropic stream', 'message stopped with unfinished blocks')
        yield { type: 'response', usage }
        return
      case 'ping':
      case 'ignored':
        break
    }
  }
  signal?.throwIfAborted()
  throw new ProviderDataError('Anthropic stream', 'stream ended before message_stop')
}

type AnthropicToolBlock = Extract<AnthropicContentBlock, { type: 'tool_use' }>
type AnthropicBlockState = Pick<AnthropicToolBlock, 'type' | 'id' | 'name'> & {
  initialInput: AnthropicToolBlock['input']
  inputJson: string
} | {
  type: Exclude<AnthropicContentBlock['type'], 'tool_use'>
}

function inferenceItemsToAnthropicMessages(
  items: InferenceItem[]
): AnthropicMessage[] {
  const messages: AnthropicMessage[] = []

  const append = (
    role: AnthropicMessage['role'],
    content: AnthropicContentBlock[]
  ) => {
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
        append(
          'assistant',
          [{
            type: 'tool_use',
            id: item.toolUseId,
            name: item.toolName,
            input: item.input ?? {}
          }]
        )
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

function userContentToAnthropic(
  content: UserContentBlock[]
): AnthropicContentBlock[] {
  return content.flatMap((block): AnthropicContentBlock[] => {
    if (block.type === 'text')
      return [{ type: 'text', text: block.text }]
    if (block.type === 'reference')
      return [{
        type: 'text',
        text: block.reference
      }]
    if (block.type === 'attachment')
      return [{ type: 'text', text: attachmentTag(block) }]
    // A document is a PDF the model reads natively; the file is also on the host by path.
    if (block.type === 'document')
      return [{
        type: 'document',
        source: {
          type: 'base64',
          media_type: block.source.mediaType,
          data: Buffer.from(block.source.data).toString('base64'),
        },
        title: block.source.fileName,
      }]
    // Anthropic's API has no video content type (catalog marks video unsupported); degrade defensively.
    if (block.type === 'video')
      return [{ type: 'text', text: '[video]' }]
    if (block.source.type === 'url')
      return [{
        type: 'text',
        text: `[image:${block.source.url}]`
      }]
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

function toolResultContentToAnthropic(
  output: ToolResultContentBlock[]
): AnthropicToolResultContent[] {
  return output.map((block) => {
    if (block.type === 'text')
      return { type: 'text', text: block.text }
    if (block.type === 'video')
      return {
        type: 'text',
        text: `[video:${block.source.mediaType}]`
      }
    return {
      type: 'image',
      source: {
        type: 'base64',
        media_type: block.source.mediaType,
        data: block.source.data
      },
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

function anthropicMessagesUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/messages')
    ? normalized
    : `${normalized}/messages`
}

function mergeAnthropicUsage(
  current: TokenUsage,
  usage: AnthropicUsage,
): TokenUsage {
  return {
    inputTokens: usage.input_tokens ?? current.inputTokens,
    outputTokens: usage.output_tokens ?? current.outputTokens,
    cacheReadTokens: usage.cache_read_input_tokens ?? current.cacheReadTokens,
    cacheWriteTokens: usage.cache_creation_input_tokens ?? current.cacheWriteTokens,
  }
}
