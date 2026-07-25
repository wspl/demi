import { Buffer } from 'node:buffer'
import process from 'node:process'
import { isAbortError, isRecord, normalizeBaseUrl, numberOrZero, parseJsonObject, stringOrNull } from '@demicodes/utils'
import { zeroUsage } from '@demicodes/core'
import type { TokenUsage, ToolResultContentBlock, UserContentBlock } from '@demicodes/core'
import {
  authStatusFromKey,
  defineProvider,
  httpRequestFailedEvent,
  normalizeErrorCode,
  providerErrorFromUnknown,
  withProviderId,
  type AgentProvider,
  type InferenceItem,
  type InferenceRequest,
  type Provider,
  type ProviderEvent,
  type ProviderModelList,
  type ProviderSelection,
  type ToolDefinition,
} from '@demicodes/provider'
import { googleDefaultModels, modelListFromGoogleModels, type GoogleModelOptions } from './models'

export type GoogleSecretResolver = () => string | Promise<string> | null | undefined
export type GoogleHeadersResolver = () => Record<string, string> | Promise<Record<string, string>>
export type GoogleFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

export interface GoogleRequestOptions {
  /** Overrides the derived maxOutputTokens (default: the model's outputLimit, else 32000). */
  maxOutputTokens?: number
  /**
   * Thinking budget used when an effort config is mapped onto Gemini's
   * `thinkingBudget` knob. Merged over the built-in ladder
   * (low 4k / medium 16k / high 32k / xhigh 64k / max 96k).
   */
  effortBudgetTokens?: Record<string, number>
  /**
   * Ask the API to stream thought summaries. On by default: without it the model
   * still thinks (and still bills `thoughtsTokenCount`) but the product has
   * nothing to show for the pause.
   */
  includeThoughts?: boolean
  extraBody?: Record<string, unknown>
}

export interface GoogleProviderOptions {
  id?: string
  displayName?: string
  envPrefix?: string
  baseUrl?: string
  apiKey?: GoogleSecretResolver
  headers?: GoogleHeadersResolver
  models?: GoogleModelOptions[]
  defaultModelId?: string
  request?: GoogleRequestOptions
  fetch?: GoogleFetch
}

interface GoogleRuntimeOptions {
  baseUrl: string
  apiKey: GoogleSecretResolver
  headers?: GoogleHeadersResolver
  request?: GoogleRequestOptions
  fetch: GoogleFetch
}

const DEFAULT_GOOGLE_BASE_URL = 'https://generativelanguage.googleapis.com/v1beta'
const DEFAULT_MAX_OUTPUT_TOKENS = 32_000
const DEFAULT_EFFORT_BUDGET_TOKENS: Record<string, number> = {
  low: 4_096,
  medium: 16_384,
  high: 32_768,
  xhigh: 65_536,
  max: 98_304,
}

export class GoogleProvider implements AgentProvider {
  constructor(private readonly options: GoogleRuntimeOptions) {}

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
      if (!apiKey && !headers.has('x-goog-api-key')) {
        yield { type: 'error', message: 'Google API key is missing', code: 'auth_missing' }
        return
      }
    } catch (error) {
      yield providerErrorFromUnknown(error, apiKey)
      return
    }

    try {
      const response = await this.options.fetch(googleStreamUrl(this.options.baseUrl, request.modelId), {
        method: 'POST',
        headers,
        body: JSON.stringify(buildGoogleGenerateContentBody(request, this.options.request)),
        signal: request.cancel,
      })
      if (!response.ok) {
        yield await httpRequestFailedEvent(response, apiKey, 'Google')
        return
      }
      yield* mapGoogleContentStream(readServerSentEvents(response.body, request.cancel), request.cancel)
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
    if (apiKey) headers.set('x-goog-api-key', apiKey)
    return headers
  }
}

export function createGoogleProvider(options: GoogleProviderOptions = {}): Provider {
  const id = options.id ?? 'google'
  const displayName = options.displayName ?? 'Google Gemini'
  const envPrefix = options.envPrefix ?? 'GOOGLE'
  const baseUrl = normalizeBaseUrl(options.baseUrl ?? process.env[`${envPrefix}_BASE_URL`] ?? DEFAULT_GOOGLE_BASE_URL)
  const apiKey = options.apiKey ?? (() => process.env[`${envPrefix}_API_KEY`])
  const fetchImpl = options.fetch ?? fetch
  const modelList = (): ProviderModelList =>
    options.models
      ? modelListFromGoogleModels(options.models, { providerId: id, defaultModelId: options.defaultModelId ?? null })
      : withProviderId(googleDefaultModels(id), id)
  const runtimeOptions: GoogleRuntimeOptions = {
    baseUrl,
    apiKey,
    headers: options.headers,
    request: options.request,
    fetch: fetchImpl,
  }

  return defineProvider({
    id,
    displayName,
    auth: { status: () => authStatusFromKey(apiKey, options.headers, 'x-goog-api-key', 'Google') },
    state: () => ({ status: 'ready', message: 'Uses the Gemini generateContent API' }),
    listModels: modelList,
    createRuntime: (selection: ProviderSelection) => {
      const outputLimit = modelList().models.find((model) => model.id === selection.model.model.id)?.outputLimit ?? null
      const request: GoogleRequestOptions = {
        ...options.request,
        maxOutputTokens: options.request?.maxOutputTokens ?? outputLimit ?? undefined,
      }
      return new GoogleProvider({ ...runtimeOptions, request })
    },
  })
}

// ── request body ────────────────────────────────────────────────────

export interface GoogleGenerateContentBody {
  contents: GoogleContent[]
  systemInstruction?: { parts: GooglePart[] }
  tools?: [{ functionDeclarations: GoogleFunctionDeclaration[] }]
  generationConfig?: GoogleGenerationConfig
  [key: string]: unknown
}

export interface GoogleGenerationConfig {
  maxOutputTokens?: number
  thinkingConfig?: { includeThoughts?: boolean; thinkingBudget?: number }
}

export interface GoogleContent {
  role: 'user' | 'model'
  parts: GooglePart[]
}

export type GooglePart =
  | { text: string; thought?: boolean; thoughtSignature?: string }
  | { inlineData: { mimeType: string; data: string } }
  | { fileData: { mimeType?: string; fileUri: string } }
  | { functionCall: { name: string; args: unknown; id?: string }; thoughtSignature?: string }
  | { functionResponse: { name: string; id?: string; response: Record<string, unknown> } }

export interface GoogleFunctionDeclaration {
  name: string
  description: string
  parameters: Record<string, unknown>
}

export function buildGoogleGenerateContentBody(
  request: InferenceRequest,
  options: GoogleRequestOptions | undefined,
): GoogleGenerateContentBody {
  const body: GoogleGenerateContentBody = {
    contents: inferenceItemsToGoogleContents(request.items),
  }
  if (request.systemPrompt.trim()) body.systemInstruction = { parts: [{ text: request.systemPrompt }] }
  if (request.tools.length > 0) {
    body.tools = [{ functionDeclarations: request.tools.map(toolToGoogleFunctionDeclaration) }]
  }
  const generationConfig: GoogleGenerationConfig = {}
  const maxOutputTokens = options?.maxOutputTokens ?? DEFAULT_MAX_OUTPUT_TOKENS
  if (maxOutputTokens > 0) generationConfig.maxOutputTokens = maxOutputTokens
  const thinkingConfig = googleThinkingConfig(request.thinking, options)
  if (thinkingConfig) generationConfig.thinkingConfig = thinkingConfig
  if (Object.keys(generationConfig).length > 0) body.generationConfig = generationConfig
  if (options?.extraBody) Object.assign(body, options.extraBody)
  return body
}

/**
 * Maps any thinking config onto Gemini's `thinkingBudget`: budget configs pass
 * through, effort configs resolve through the ladder (caller-overridable),
 * disabled pins the budget to 0. `includeThoughts` defaults on — the model
 * thinks either way and bills for it, so hiding the summaries only costs the
 * product its "thinking…" surface.
 */
export function googleThinkingConfig(
  thinking: InferenceRequest['thinking'],
  options: GoogleRequestOptions | undefined,
): GoogleGenerationConfig['thinkingConfig'] | null {
  const includeThoughts = options?.includeThoughts ?? true
  if (!thinking) return includeThoughts ? { includeThoughts: true } : null
  if (thinking.type === 'disabled') return { includeThoughts: false, thinkingBudget: 0 }
  const budget =
    thinking.type === 'budget'
      ? thinking.budgetTokens
      : (options?.effortBudgetTokens?.[thinking.effort] ??
        DEFAULT_EFFORT_BUDGET_TOKENS[thinking.effort] ??
        DEFAULT_EFFORT_BUDGET_TOKENS.medium!)
  return { includeThoughts, thinkingBudget: Math.max(0, budget) }
}

/**
 * Rebuilds the conversation as Gemini `contents`.
 *
 * The subtle part is `thoughtSignature`. Gemini hands one back on every
 * `functionCall` part and REQUIRES it verbatim when that call is replayed —
 * without it the API rejects the whole request ("Function call is missing a
 * thought_signature in functionCall parts"), so an agent loop cannot survive
 * its own second turn. demi's item model has no signature slot on `tool_use`,
 * but it has one on `assistant_thinking`, and the transcript keeps items in
 * order: the provider emits `thinking_start` + `thinking_signature` right
 * before each `tool_call_requested`, so on replay the thinking item sitting
 * immediately in front of a `tool_use` carries that call's signature. That
 * keeps the signature in the persisted transcript (a runtime-local map would
 * evaporate on session resume and 400 forever after) without touching core.
 */
export function inferenceItemsToGoogleContents(items: InferenceItem[]): GoogleContent[] {
  const contents: GoogleContent[] = []
  // Gemini's functionResponse needs the tool NAME, which only the matching
  // tool_use item carries; remember it as we walk forward.
  const toolNames = new Map<string, string>()
  let pendingSignature: string | null = null

  const append = (role: GoogleContent['role'], parts: GooglePart[]) => {
    if (parts.length === 0) return
    const last = contents[contents.length - 1]
    if (last?.role === role) {
      last.parts.push(...parts)
      return
    }
    contents.push({ role, parts })
  }

  for (const item of items) {
    switch (item.type) {
      case 'user_message':
      case 'user_steer':
        append('user', userContentToGoogle(item.content))
        pendingSignature = null
        break
      case 'assistant_text':
        append('model', [{ text: item.text }])
        pendingSignature = null
        break
      case 'assistant_thinking':
        // Thought text is not replayed (Gemini re-derives it); only the
        // signature matters, and only for the call that follows.
        pendingSignature = item.signature
        break
      case 'assistant_redacted_thinking':
        break
      case 'tool_use': {
        toolNames.set(item.toolUseId, item.toolName)
        const call: GooglePart = {
          functionCall: { name: item.toolName, args: item.input ?? {}, id: item.toolUseId },
        }
        if (pendingSignature) call.thoughtSignature = pendingSignature
        append('model', [call])
        pendingSignature = null
        break
      }
      case 'tool_result':
        append('user', toolResultToGoogle(item.toolUseId, toolNames.get(item.toolUseId) ?? 'tool', item.output))
        pendingSignature = null
        break
    }
  }

  return contents
}

function userContentToGoogle(content: UserContentBlock[]): GooglePart[] {
  return content.flatMap((block): GooglePart[] => {
    if (block.type === 'text') return [{ text: block.text }]
    if (block.type === 'reference') return [{ text: block.reference }]
    // Documents ride inline too (the API reads PDFs and text natively) rather
    // than collapsing to a "[document:…]" placeholder.
    if (block.type === 'document') return [inlinePart(block.source.mediaType, block.source.data)]
    // Video and images share one shape. Video is first-class here — the model
    // reads the frames AND the audio track — so nothing has to be degraded the
    // way an adapter without a video part must.
    if (block.source.type === 'url') return [{ fileData: { fileUri: block.source.url } }]
    return [inlinePart(block.source.mediaType, block.source.data)]
  })
}

function inlinePart(mimeType: string, data: Uint8Array): GooglePart {
  return { inlineData: { mimeType, data: Buffer.from(data).toString('base64') } }
}

/**
 * `functionResponse.response` is a JSON object, so images and video a tool
 * returns cannot ride inside it. They follow as sibling inline parts in the
 * same user content — which is how the model ends up actually seeing what a
 * command printed.
 */
function toolResultToGoogle(toolUseId: string, toolName: string, output: ToolResultContentBlock[]): GooglePart[] {
  const text = output
    .filter((block): block is Extract<ToolResultContentBlock, { type: 'text' }> => block.type === 'text')
    .map((block) => block.text)
    .join('\n')
  const media = output
    .filter((block) => block.type !== 'text')
    .map((block): GooglePart => ({ inlineData: { mimeType: block.source.mediaType, data: block.source.data } }))
  return [
    { functionResponse: { name: toolName, id: toolUseId, response: { output: text } } },
    ...media,
  ]
}

function toolToGoogleFunctionDeclaration(tool: ToolDefinition): GoogleFunctionDeclaration {
  return {
    name: tool.name,
    description: tool.description,
    parameters: tool.inputSchema,
  }
}

// ── response stream ─────────────────────────────────────────────────

export async function* mapGoogleContentStream(
  events: AsyncIterable<ServerSentEvent>,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  let usage = zeroUsage()
  let thinkingOpen = false

  for await (const event of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    for (const data of event.data) {
      const value = parseJsonObject(data)
      if (!value) continue

      if (isRecord(value.error)) {
        const message = stringOrNull(value.error.message) ?? 'Google API stream error'
        yield { type: 'error', message, code: normalizeErrorCode(stringOrNull(value.error.status), message) }
        return
      }

      if (isRecord(value.usageMetadata)) usage = googleUsage(value.usageMetadata)

      const candidates = Array.isArray(value.candidates) ? value.candidates : []
      for (const candidate of candidates) {
        if (!isRecord(candidate)) continue
        const content = isRecord(candidate.content) ? candidate.content : null
        const parts = content && Array.isArray(content.parts) ? content.parts : []
        for (const part of parts) {
          if (!isRecord(part)) continue

          const functionCall = isRecord(part.functionCall) ? part.functionCall : null
          if (functionCall) {
            const signature = stringOrNull(part.thoughtSignature)
            if (signature) {
              // Park the signature on a thinking item so it survives into the
              // transcript directly in front of this call (see
              // inferenceItemsToGoogleContents).
              if (!thinkingOpen) yield { type: 'thinking_start' }
              thinkingOpen = true
              yield { type: 'thinking_signature', signature }
            }
            yield {
              type: 'tool_call_requested',
              toolUseId: stringOrNull(functionCall.id) ?? `${stringOrNull(functionCall.name) ?? 'tool'}_${nextFallbackToolId()}`,
              toolName: stringOrNull(functionCall.name) ?? '',
              input: functionCall.args ?? {},
            }
            thinkingOpen = false
            continue
          }

          const text = stringOrNull(part.text)
          if (part.thought === true) {
            if (!thinkingOpen) {
              yield { type: 'thinking_start' }
              thinkingOpen = true
            }
            if (text) yield { type: 'thinking_delta', text }
            continue
          }

          const signature = stringOrNull(part.thoughtSignature)
          if (signature && thinkingOpen) yield { type: 'thinking_signature', signature }
          if (text) {
            thinkingOpen = false
            yield { type: 'text_delta', text }
          }
        }
      }
    }
  }

  yield { type: 'response', usage }
}

let sequence = 0
/** Fallback tool-call id when the API omits one; only needs to be unique per process. */
function nextFallbackToolId(): string {
  sequence += 1
  return String(sequence)
}

/**
 * Thinking is billed separately from the visible answer (`thoughtsTokenCount`
 * alongside `candidatesTokenCount`); both are output tokens, so the agent's
 * context estimate has to count them together.
 */
function googleUsage(usageMetadata: Record<string, unknown>): TokenUsage {
  return {
    inputTokens: numberOrZero(usageMetadata.promptTokenCount),
    outputTokens: numberOrZero(usageMetadata.candidatesTokenCount) + numberOrZero(usageMetadata.thoughtsTokenCount),
    cacheReadTokens: numberOrZero(usageMetadata.cachedContentTokenCount),
    cacheWriteTokens: 0,
  }
}

// ── transport ───────────────────────────────────────────────────────

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

function googleStreamUrl(baseUrl: string, modelId: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return `${normalized}/models/${encodeURIComponent(modelId)}:streamGenerateContent?alt=sse`
}
