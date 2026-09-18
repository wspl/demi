import { attachmentTag } from '@demicodes/core'
import {
  isAbortError,
  normalizeBaseUrl,
  parseJsonOrString
} from '@demicodes/utils'
import { Buffer } from 'node:buffer'
import process from 'node:process'
import { z } from 'zod'
import { zeroUsage } from '@demicodes/core'
import type {
  TokenUsage,
  ToolResultContentBlock,
  UserContentBlock
} from '@demicodes/core'
import {
  authStatusFromKey,
  defineProvider,
  httpRequestFailedEvent,
  normalizeErrorCode,
  providerErrorFromUnknown,
  readHttpFailure,
  readServerSentEvents,
  withRetryWait,
  reportedStringSchema,
  taggedUnion,
  tokenCountSchema,
  withCatalogProviderId,
  type AgentProvider,
  type InferenceItem,
  type InferenceRequest,
  type Provider,
  type ProviderEvent,
  type ProviderModelList,
  type ServerSentEvent,
  type ToolDefinition,
} from '@demicodes/provider'
import {
  anthropicApiDefaultModels,
  modelListFromAnthropicApiModels,
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
      yield providerErrorFromUnknown(error)
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
        yield await httpRequestFailedEvent(response, 'Anthropic', readHttpFailure)
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
      yield providerErrorFromUnknown(error)
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
      ? modelListFromAnthropicApiModels(
        options.models,
        { providerId: id, defaultModelId: options.defaultModelId ?? null }
      )
      : withCatalogProviderId(anthropicApiDefaultModels(id), id)
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
    readFailure: readHttpFailure,
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

// ── response stream ─────────────────────────────────────────────────

/**
 * Token counts as the Messages API reports them: `message_start` carries the
 * input side, `message_delta` the output side.
 */
const anthropicUsageSchema = z.looseObject({
  input_tokens: tokenCountSchema,
  output_tokens: tokenCountSchema,
  cache_read_input_tokens: tokenCountSchema,
  cache_creation_input_tokens: tokenCountSchema,
})
type AnthropicUsage = z.infer<typeof anthropicUsageSchema>

/** The error object an `error` event nests under `error`. */
const anthropicErrorSchema = z.looseObject({
  type: reportedStringSchema,
  message: reportedStringSchema,
})

/** The content blocks Demi maps; any other block type decodes to null. */
const anthropicContentBlockSchema = taggedUnion({
  text: z.looseObject({
    type: z.literal('text'),
    text: z.string(),
  }),
  thinking: z.looseObject({ type: z.literal('thinking') }),
  tool_use: z.looseObject({
    type: z.literal('tool_use'),
    id: z.string().min(1),
    name: z.string().min(1),
    input: z.unknown().optional(),
  }),
})

/** The block deltas Demi maps; any other delta type decodes to null. */
const anthropicBlockDeltaSchema = taggedUnion({
  text_delta: z.looseObject({
    type: z.literal('text_delta'),
    text: z.string(),
  }),
  thinking_delta: z.looseObject({
    type: z.literal('thinking_delta'),
    thinking: z.string(),
  }),
  signature_delta: z.looseObject({
    type: z.literal('signature_delta'),
    signature: z.string(),
  }),
  input_json_delta: z.looseObject({
    type: z.literal('input_json_delta'),
    partial_json: z.string(),
  }),
})

/**
 * The Messages API stream events Demi maps; any other event type (`ping`, or
 * whatever the vendor adds next) decodes to null and is ignored. `index` ties
 * a block's start, deltas and stop together, so a block event without one is
 * a protocol error rather than a silent write to block 0.
 */
const anthropicStreamEventSchema = taggedUnion({
  message_start: z.looseObject({
    type: z.literal('message_start'),
    message: z.looseObject({ usage: anthropicUsageSchema.optional() }),
  }),
  content_block_start: z.looseObject({
    type: z.literal('content_block_start'),
    index: z.number().int(),
    content_block: anthropicContentBlockSchema,
  }),
  content_block_delta: z.looseObject({
    type: z.literal('content_block_delta'),
    index: z.number().int(),
    delta: anthropicBlockDeltaSchema,
  }),
  content_block_stop: z.looseObject({
    type: z.literal('content_block_stop'),
    index: z.number().int(),
  }),
  message_delta: z.looseObject({
    type: z.literal('message_delta'),
    usage: anthropicUsageSchema.optional(),
  }),
  message_stop: z.looseObject({ type: z.literal('message_stop') }),
  error: z.looseObject({
    type: z.literal('error'),
    message: reportedStringSchema,
    error: anthropicErrorSchema.optional(),
  }),
})

export async function* mapAnthropicMessageStream(
  events: AsyncIterable<ServerSentEvent>,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const toolBlocks = new Map<number, AnthropicToolBlock>()
  let usage = zeroUsage()

  for await (const frame of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    const event = anthropicStreamEventSchema.parse(JSON.parse(frame.data))
    if (!event)
      continue

    switch (event.type) {
      case 'error': {
        // The failure rides in a nested `error` object whose `type` is the
        // vendor's error class. A payload without that object leaves only the
        // event's own tag to classify by.
        const message = event.error?.message
          ?? event.message
          ?? 'Anthropic API stream error'
        yield withRetryWait({
          type: 'error',
          message,
          code: normalizeErrorCode(event.error?.type ?? event.type, message),
          diagnostics: { source: 'stream', upstream: frame.data },
        }, readHttpFailure)
        return
      }

      case 'message_start':
        usage = mergeAnthropicUsage(usage, event.message.usage)
        break

      case 'content_block_start': {
        const block = event.content_block
        if (!block)
          break
        if (block.type === 'tool_use') {
          toolBlocks.set(event.index, {
            id: block.id,
            name: block.name,
            initialInput: block.input,
            inputJson: '',
          })
        } else if (block.type === 'thinking') {
          yield { type: 'thinking_start' }
        } else if (block.text) {
          // A text block usually opens empty and fills through deltas.
          yield { type: 'text_delta', text: block.text }
        }
        break
      }

      case 'content_block_delta': {
        const delta = event.delta
        if (!delta)
          break
        if (delta.type === 'text_delta') {
          if (delta.text)
            yield { type: 'text_delta', text: delta.text }
        } else if (delta.type === 'thinking_delta') {
          if (delta.thinking)
            yield { type: 'thinking_delta', text: delta.thinking }
        } else if (delta.type === 'signature_delta') {
          if (delta.signature)
            yield { type: 'thinking_signature', signature: delta.signature }
        } else if (delta.type === 'input_json_delta') {
          // A tool call's input arrives as JSON text, one piece per delta.
          const block = toolBlocks.get(event.index)
          if (block)
            block.inputJson += delta.partial_json
        }
        break
      }

      case 'content_block_stop': {
        const block = toolBlocks.get(event.index)
        if (block) {
          yield {
            type: 'tool_call_requested',
            toolUseId: block.id,
            toolName: block.name,
            input: block.inputJson
              ? parseJsonOrString(block.inputJson)
              : block.initialInput ?? {},
          }
        }
        toolBlocks.delete(event.index)
        break
      }

      case 'message_delta':
        usage = mergeAnthropicUsage(usage, event.usage)
        break

      case 'message_stop':
        yield { type: 'response', usage }
        return
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

/**
 * Folds one event's counts into the running total. A stream reports each count
 * once — the input side on `message_start`, the output side on `message_delta`
 * — and repeats the others as zero or omits them, so a count that is absent or
 * zero leaves the running total alone.
 */
function mergeAnthropicUsage(
  current: TokenUsage,
  usage: AnthropicUsage | undefined
): TokenUsage {
  if (!usage)
    return current
  return {
    inputTokens: usage.input_tokens || current.inputTokens,
    outputTokens: usage.output_tokens || current.outputTokens,
    cacheReadTokens: usage.cache_read_input_tokens || current.cacheReadTokens,
    cacheWriteTokens:
      usage.cache_creation_input_tokens || current.cacheWriteTokens,
  }
}

