import { isRecord } from '@demi/utils'
import {
  applyModelPolicy,
  defineProvider,
  type AgentProvider,
  type InferenceRequest,
  type ModelPolicy,
  type Provider,
  type ProviderEvent,
} from '@demi/provider'
import {
  CodexAuthError,
  FileCodexAuthStore,
  redactSecretText,
  type CodexAuthStore,
  type CodexResolvedAuth,
} from './auth'
import { listCodexModels } from './models'
import { buildCodexResponsesRequestBody, mapCodexResponseEvents } from './responses'
import {
  CodexHttpError,
  codexResponsesUrl,
  codexWebSocketUrl,
  createCodexTransport,
  type CodexResponsesTransport,
} from './transport'
import type { CodexTransportMode } from './types'

export interface CodexProviderConfig {
  codexHome?: string
  baseUrl?: string
  transport?: CodexTransportMode
  headers?: Record<string, string>
  userAgent?: string
  maxRetries?: number
  retryBaseDelayMs?: number
  headerTimeoutMs?: number
  websocketConnectTimeoutMs?: number
  streamIdleTimeoutMs?: number
  clientVersion?: string
}

export interface CodexProviderOptions extends CodexProviderConfig {
  id?: string
  displayName?: string
  models?: ModelPolicy
}

export interface CodexRuntimeOptions extends CodexProviderConfig {
  authStore?: CodexAuthStore
  transportImpl?: CodexResponsesTransport
}

const DEFAULT_CHATGPT_CODEX_BASE_URL = 'https://chatgpt.com/backend-api'
const DEFAULT_OPENAI_BASE_URL = 'https://api.openai.com/v1'
const DEFAULT_MAX_RETRIES = 2
const DEFAULT_RETRY_BASE_DELAY_MS = 250
const DEFAULT_SSE_HEADER_TIMEOUT_MS = 20_000
const DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS = 10_000

export class CodexProvider implements AgentProvider {
  private readonly config: Required<Omit<CodexProviderConfig, 'codexHome' | 'baseUrl' | 'headers' | 'clientVersion'>> &
    Pick<CodexProviderConfig, 'codexHome' | 'baseUrl' | 'headers' | 'clientVersion'>
  private readonly authStore: CodexAuthStore
  private readonly transport: CodexResponsesTransport

  constructor(options: CodexRuntimeOptions = {}) {
    this.config = {
      codexHome: options.codexHome,
      baseUrl: options.baseUrl,
      transport: options.transport ?? 'auto',
      headers: options.headers,
      userAgent: options.userAgent ?? defaultUserAgent(),
      maxRetries: options.maxRetries ?? DEFAULT_MAX_RETRIES,
      retryBaseDelayMs: options.retryBaseDelayMs ?? DEFAULT_RETRY_BASE_DELAY_MS,
      headerTimeoutMs: options.headerTimeoutMs ?? DEFAULT_SSE_HEADER_TIMEOUT_MS,
      websocketConnectTimeoutMs: options.websocketConnectTimeoutMs ?? DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS,
      streamIdleTimeoutMs: options.streamIdleTimeoutMs ?? 0,
      clientVersion: options.clientVersion,
    }
    this.authStore = options.authStore ?? new FileCodexAuthStore({ codexHome: options.codexHome })
    this.transport = options.transportImpl ?? createCodexTransport(this.config.transport)
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const body = buildCodexResponsesRequestBody(request)
    let forceRefresh = false
    let retryAttempt = 0

    while (retryAttempt <= this.config.maxRetries) {
      try {
        const auth = await this.authStore.resolveAuth({ forceRefresh })
        const url = responsesUrlForAuth(auth, this.config.baseUrl)
        const headers = buildCodexHeaders(auth, request, this.config)
        const stream = this.transport.stream({
          url,
          websocketUrl: codexWebSocketUrl(url),
          headers,
          body,
          signal: request.cancel,
          headerTimeoutMs: this.config.headerTimeoutMs,
          websocketConnectTimeoutMs: this.config.websocketConnectTimeoutMs,
          streamIdleTimeoutMs: this.config.streamIdleTimeoutMs || undefined,
        })
        yield* mapCodexResponseEvents(stream)
        return
      } catch (error) {
        if (request.cancel.aborted) {
          yield { type: 'abort' }
          return
        }
        if (error instanceof CodexHttpError && error.status === 401 && !forceRefresh) {
          forceRefresh = true
          continue
        }
        if (error instanceof CodexHttpError && isRetryableHttpStatus(error.status) && retryAttempt < this.config.maxRetries) {
          await sleep(this.config.retryBaseDelayMs * 2 ** retryAttempt, request.cancel)
          retryAttempt += 1
          continue
        }
        yield providerErrorFromUnknown(error)
        return
      }
    }
  }
}

export function createCodexProvider(options: CodexProviderOptions = {}): Provider {
  const id = options.id ?? 'codex'
  const displayName = options.displayName ?? 'Codex'
  const runtimeOptions: CodexRuntimeOptions = {
    codexHome: options.codexHome,
    baseUrl: options.baseUrl,
    transport: options.transport,
    headers: options.headers,
    userAgent: options.userAgent,
    maxRetries: options.maxRetries,
    retryBaseDelayMs: options.retryBaseDelayMs,
    headerTimeoutMs: options.headerTimeoutMs,
    websocketConnectTimeoutMs: options.websocketConnectTimeoutMs,
    streamIdleTimeoutMs: options.streamIdleTimeoutMs,
    clientVersion: options.clientVersion,
  }

  return defineProvider({
    id,
    displayName,
    auth: { status: () => new FileCodexAuthStore({ codexHome: options.codexHome }).status() },
    state: () => ({ status: 'ready', message: 'Uses official Codex auth storage' }),
    listModels: async () => {
      const catalog = await listCodexModels(runtimeOptions)
      return applyModelPolicy(catalog, id, options.models)
    },
    createRuntime: () => new CodexProvider(runtimeOptions),
  })
}

export function parseCodexProviderConfig(config: unknown): CodexProviderConfig {
  if (config === undefined || config === null) return {}
  if (!isRecord(config)) throw new Error('Codex provider config must be an object')

  const parsed: CodexProviderConfig = {}
  if (config.codexHome !== undefined) parsed.codexHome = expectString(config.codexHome, 'codexHome')
  if (config.baseUrl !== undefined) parsed.baseUrl = expectString(config.baseUrl, 'baseUrl')
  if (config.transport !== undefined) {
    if (config.transport !== 'auto' && config.transport !== 'sse' && config.transport !== 'websocket') {
      throw new Error('Codex provider config field "transport" must be auto, sse, or websocket')
    }
    parsed.transport = config.transport
  }
  if (config.headers !== undefined) parsed.headers = expectStringRecord(config.headers, 'headers')
  if (config.userAgent !== undefined) parsed.userAgent = expectString(config.userAgent, 'userAgent')
  if (config.maxRetries !== undefined) parsed.maxRetries = expectNumber(config.maxRetries, 'maxRetries')
  if (config.retryBaseDelayMs !== undefined) parsed.retryBaseDelayMs = expectNumber(config.retryBaseDelayMs, 'retryBaseDelayMs')
  if (config.headerTimeoutMs !== undefined) parsed.headerTimeoutMs = expectNumber(config.headerTimeoutMs, 'headerTimeoutMs')
  if (config.websocketConnectTimeoutMs !== undefined) {
    parsed.websocketConnectTimeoutMs = expectNumber(config.websocketConnectTimeoutMs, 'websocketConnectTimeoutMs')
  }
  if (config.streamIdleTimeoutMs !== undefined) parsed.streamIdleTimeoutMs = expectNumber(config.streamIdleTimeoutMs, 'streamIdleTimeoutMs')
  if (config.clientVersion !== undefined) parsed.clientVersion = expectString(config.clientVersion, 'clientVersion')
  return parsed
}

export function buildCodexHeaders(
  auth: CodexResolvedAuth,
  request: InferenceRequest,
  config: Pick<CodexProviderConfig, 'headers' | 'userAgent'>,
): Headers {
  const headers = new Headers(config.headers)
  if (auth.kind === 'agentIdentity') headers.set('Authorization', auth.authorization)
  else headers.set('Authorization', `Bearer ${auth.kind === 'apiKey' ? auth.apiKey : auth.accessToken}`)

  if (auth.kind !== 'apiKey' && auth.accountId) headers.set('ChatGPT-Account-ID', auth.accountId)
  if (auth.kind !== 'apiKey' && 'isFedrampAccount' in auth && auth.isFedrampAccount) headers.set('X-OpenAI-Fedramp', 'true')
  headers.set('User-Agent', config.userAgent ?? defaultUserAgent())
  headers.set('OpenAI-Beta', 'responses=experimental')
  headers.set('accept', 'text/event-stream')
  headers.set('content-type', 'application/json')
  headers.set('session-id', request.sessionId)
  headers.set('thread-id', request.sessionId)
  headers.set('x-client-request-id', request.requestId)
  return headers
}

export function responsesUrlForAuth(auth: CodexResolvedAuth, baseUrl?: string): string {
  if (auth.kind === 'apiKey') return openAiResponsesUrl(baseUrl ?? DEFAULT_OPENAI_BASE_URL)
  return codexResponsesUrl(baseUrl ?? DEFAULT_CHATGPT_CODEX_BASE_URL)
}

function openAiResponsesUrl(baseUrl: string): string {
  const normalized = baseUrl.replace(/\/+$/, '')
  return normalized.endsWith('/responses') ? normalized : `${normalized}/responses`
}

function providerErrorFromUnknown(error: unknown): ProviderEvent {
  if (error instanceof CodexAuthError) return { type: 'error', message: redactSecretText(error.message), code: error.code }
  if (error instanceof CodexHttpError) {
    return {
      type: 'error',
      message: redactSecretText(error.message),
      code: httpErrorCode(error.status, error.message),
    }
  }
  const message = error instanceof Error ? error.message : String(error)
  return { type: 'error', message: redactSecretText(message), code: null }
}

function httpErrorCode(status: number, message: string): string | null {
  if (status === 401 || status === 403) return 'auth_expired'
  if (status === 408 || status === 409 || status === 425 || status === 429 || status >= 500) return 'rate_limit'
  if (status === 400 && /context|too long|token/i.test(message)) return 'context_length_exceeded'
  return null
}

function isRetryableHttpStatus(status: number): boolean {
  return status === 408 || status === 409 || status === 425 || status === 429 || status === 500 || status === 502 || status === 503 || status === 504
}

function expectString(value: unknown, field: string): string {
  if (typeof value !== 'string') throw new Error(`Codex provider config field "${field}" must be a string`)
  return value
}

function expectNumber(value: unknown, field: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new Error(`Codex provider config field "${field}" must be a finite number`)
  }
  return value
}

function expectStringRecord(value: unknown, field: string): Record<string, string> {
  if (!isRecord(value)) throw new Error(`Codex provider config field "${field}" must be an object`)
  const out: Record<string, string> = {}
  for (const [key, nested] of Object.entries(value)) {
    if (typeof nested !== 'string') throw new Error(`Codex provider config field "${field}.${key}" must be a string`)
    out[key] = nested
  }
  return out
}

function defaultUserAgent(): string {
  return `demi-codex-provider/0.0.0 (${process.platform}; ${process.arch})`
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'))
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(resolve, ms)
    signal.addEventListener(
      'abort',
      () => {
        clearTimeout(timeout)
        reject(new DOMException('Aborted', 'AbortError'))
      },
      { once: true },
    )
  })
}
