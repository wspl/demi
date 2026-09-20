import { z } from 'zod'
import {
  applyModelPolicy,
  clampPromptCacheKey,
  defineProvider,
  httpErrorCode,
  mapResponsesEvents,
  httpFailureRecord,
  normalizeErrorCode,
  quotaSnapshotFile,
  withRetryWait,
  type AgentProvider,
  type InferenceRequest,
  type ModelPolicy,
  type Provider,
  type ProviderEvent,
  type ProviderQuota,
  type ProviderQuotaObserver,
} from '@demicodes/provider'
import {
  CodexAuthError,
  FileCodexAuthStore,
  redactCodexSecretText,
  type CodexAuthStore,
  type CodexResolvedAuth,
} from './auth'
import {
  createCodexCredentials,
  openCodexCredentialPool,
  PoolAwareCodexAuthStore
} from './credentials'
import { readCodexFailure } from './failure'
import { listCodexModels } from './models'
import { createCodexQuota } from './quota'
import { buildCodexResponsesRequestBody } from './responses'
import {
  CodexHttpError,
  codexResponsesUrl,
  codexWebSocketUrl,
  createCodexTransport,
  decodeCodexHttpErrorBody,
  type CodexResponsesTransport,
} from './transport'
import { CODEX_TRANSPORT_MODES } from './types'

/**
 * The Codex provider's configuration, as a config file states it. Unknown keys
 * are rejected: a misspelled key would otherwise be silently ignored.
 */
export const codexProviderConfigSchema = z.strictObject({
  codexHome: z.string().optional(),
  baseUrl: z.string().optional(),
  transport: z.enum(CODEX_TRANSPORT_MODES).optional(),
  headers: z.record(z.string(), z.string()).optional(),
  userAgent: z.string().optional(),
  headerTimeoutMs: z.number().finite().optional(),
  websocketConnectTimeoutMs: z.number().finite().optional(),
  streamIdleTimeoutMs: z.number().finite().optional(),
  clientVersion: z.string().optional(),
})

export type CodexProviderConfig = z.infer<typeof codexProviderConfigSchema>

export interface CodexProviderOptions extends CodexProviderConfig {
  id?: string
  displayName?: string
  models?: ModelPolicy
  authStore?: CodexAuthStore
  /** Demi state root for credential pool (`$DEMI_HOME` / `~/.demi`). */
  stateDir?: string
  /**
   * When true (default if `authStore` is not provided), attach multi-credential
   * pool + global switch under `provider.credentials`.
   */
  credentials?: boolean
}

export interface CodexRuntimeOptions extends CodexProviderConfig {
  authStore?: CodexAuthStore
  transportImpl?: CodexResponsesTransport
  /**
   * Shared with the public Provider shell so inference can passively update
   * quota.
   */
  quota?: ProviderQuota
}

/** Names Codex in the errors the shared Responses mapper reports. */
const CODEX_VENDOR_LABEL = 'Codex'
const DEFAULT_CHATGPT_CODEX_BASE_URL = 'https://chatgpt.com/backend-api'
const DEFAULT_OPENAI_BASE_URL = 'https://api.openai.com/v1'
const DEFAULT_SSE_HEADER_TIMEOUT_MS = 20_000
const DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS = 10_000

export class CodexProvider implements AgentProvider {
  private readonly cloneOptions: CodexRuntimeOptions
  private readonly config: Required<Omit<CodexProviderConfig, 'codexHome'
    | 'baseUrl'
    | 'headers'
    | 'clientVersion'>> &
    Pick<CodexProviderConfig, 'codexHome'
      | 'baseUrl'
      | 'headers'
      | 'clientVersion'>
  private readonly authStore: CodexAuthStore
  private readonly transport: CodexResponsesTransport
  private readonly quota: ProviderQuota | null

  constructor(options: CodexRuntimeOptions = {}) {
    this.cloneOptions = options
    this.config = {
      codexHome: options.codexHome,
      baseUrl: options.baseUrl,
      transport: options.transport ?? 'auto',
      headers: options.headers,
      userAgent: options.userAgent ?? defaultUserAgent(),
      headerTimeoutMs: options.headerTimeoutMs ?? DEFAULT_SSE_HEADER_TIMEOUT_MS,
      websocketConnectTimeoutMs: options.websocketConnectTimeoutMs
        ?? DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS,
      streamIdleTimeoutMs: options.streamIdleTimeoutMs ?? 0,
      clientVersion: options.clientVersion,
    }
    this.authStore = options.authStore ?? new FileCodexAuthStore({
      codexHome: options.codexHome
    })
    this.transport = options.transportImpl
      ?? createCodexTransport(this.config.transport)
    this.quota = options.quota ?? null
  }

  clone(): AgentProvider {
    return new CodexProvider(this.cloneOptions)
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const body = buildCodexResponsesRequestBody(request)
    let forceRefresh = false

    while (true) {
      const observeQuota = this.quota?.captureObserver()
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
          onHttpResponse: (response) => {
            this.observeQuotaResponse(
              observeQuota,
              { headers: response.headers, status: response.status }
            )
          },
        })
        yield* mapResponsesEvents(stream, CODEX_VENDOR_LABEL, readCodexFailure)
        return
      } catch (error) {
        if (error instanceof CodexHttpError) {
          this.observeQuotaResponse(
            observeQuota,
            { headers: error.headers, status: error.status }
          )
        }
        if (request.cancel.aborted) {
          yield { type: 'abort' }
          return
        }
        if (error instanceof CodexHttpError && error.status === 401
          && !forceRefresh) {
          forceRefresh = true
          continue
        }
        yield codexErrorEvent(error)
        return
      }
    }
  }

  private observeQuotaResponse(
    observe: ProviderQuotaObserver | undefined,
    input: {
      headers: Headers;
      status: number
    }
  ): void {
    try {
      observe?.(input)
    } catch {
      // Quota observation must never break inference.
    }
  }
}

export function createCodexProvider(
  options: CodexProviderOptions = {}
): Provider {
  const id = options.id ?? 'codex'
  const displayName = options.displayName ?? 'Codex'
  const enableCredentials = options.credentials
    ?? options.authStore === undefined
  const pool = !options.authStore
    && enableCredentials ? openCodexCredentialPool(
    {
      stateDir: options.stateDir
    }
  ) : null

  const authStore: CodexAuthStore =
    options.authStore ??
    (pool
      ? new PoolAwareCodexAuthStore(pool, { codexHome: options.codexHome })
      : new FileCodexAuthStore({ codexHome: options.codexHome }))

  const runtimeOptions: CodexRuntimeOptions = {
    authStore,
    codexHome: options.codexHome,
    baseUrl: options.baseUrl,
    transport: options.transport,
    headers: options.headers,
    userAgent: options.userAgent,
    headerTimeoutMs: options.headerTimeoutMs,
    websocketConnectTimeoutMs: options.websocketConnectTimeoutMs,
    streamIdleTimeoutMs: options.streamIdleTimeoutMs,
    clientVersion: options.clientVersion,
  }
  const quota = createCodexQuota({
    providerId: id,
    codexHome: options.codexHome,
    baseUrl: options.baseUrl,
    authStore,
    userAgent: options.userAgent,
    snapshotFile: quotaSnapshotFile(options.stateDir),
  })
  runtimeOptions.quota = quota

  const credentialsApi = pool
    ? createCodexCredentials(
      pool,
      authStore,
      { codexHome: options.codexHome, quota }
    )
    : undefined

  return defineProvider({
    id,
    displayName,
    readFailure: readCodexFailure,
    auth: { status: () => authStore.status() },
    quota,
    ...(credentialsApi ? { credentials: credentialsApi } : {}),
    state: () => ({
      status: 'ready',
      message: credentialsApi
        ? 'Uses Codex auth + demi credential pool'
        : 'Uses official Codex auth storage',
    }),
    listModels: async (listOptions) => {
      const catalog = await listCodexModels({
        ...runtimeOptions,
        ...listOptions
      })
      return applyModelPolicy(catalog, id, options.models)
    },
    createRuntime: () => new CodexProvider(runtimeOptions),
  })
}

export function parseCodexProviderConfig(config: unknown): CodexProviderConfig {
  return codexProviderConfigSchema.parse(config ?? {})
}

export function buildCodexHeaders(
  auth: CodexResolvedAuth,
  request: Pick<InferenceRequest, 'sessionId' | 'requestId'>,
  config: Pick<CodexProviderConfig, 'headers' | 'userAgent'>,
): Headers {
  const headers = new Headers(config.headers)
  if (auth.kind === 'agentIdentity') headers.set(
    'Authorization',
    auth.authorization
  )
  else headers.set(
    'Authorization',
    `Bearer ${auth.kind === 'apiKey' ? auth.apiKey : auth.accessToken}`
  )

  if (auth.kind !== 'apiKey' && auth.accountId)
    headers.set(
      'ChatGPT-Account-ID',
      auth.accountId
    )
  if (auth.kind !== 'apiKey' && 'isFedrampAccount' in auth
    && auth.isFedrampAccount) headers.set(
    'X-OpenAI-Fedramp',
    'true'
  )
  headers.set('User-Agent', config.userAgent ?? defaultUserAgent())
  headers.set('OpenAI-Beta', 'responses=experimental')
  headers.set('accept', 'text/event-stream')
  headers.set('content-type', 'application/json')
  // The backend reads the session identity from these headers (overriding the
  // body's prompt_cache_key), so they need the same 64-character clamp.
  const sessionId = clampPromptCacheKey(request.sessionId)
  headers.set('session-id', sessionId)
  headers.set('thread-id', sessionId)
  headers.set('x-client-request-id', request.requestId)
  return headers
}

export function responsesUrlForAuth(
  auth: CodexResolvedAuth,
  baseUrl?: string
): string {
  if (auth.kind === 'apiKey')
    return openAiResponsesUrl(baseUrl
      ?? DEFAULT_OPENAI_BASE_URL)
  return codexResponsesUrl(baseUrl ?? DEFAULT_CHATGPT_CODEX_BASE_URL)
}

function openAiResponsesUrl(baseUrl: string): string {
  const normalized = baseUrl.replace(/\/+$/, '')
  return normalized.endsWith('/responses')
    ? normalized
    : `${normalized}/responses`
}

/**
 * A Codex failure as a provider error. An auth failure is Demi's own and keeps
 * the redaction its credential files need. An HTTP failure keeps the vendor's
 * response as its record and its wait from `readCodexFailure`. Anything else
 * is a transport failure with no answer from the vendor.
 */
function codexErrorEvent(error: unknown): ProviderEvent {
  if (error instanceof CodexAuthError)
    return {
      type: 'error',
      message: redactCodexSecretText(error.message),
      code: error.code
    }
  if (error instanceof CodexHttpError) {
    const body = decodeCodexHttpErrorBody(error.responseText)
    const providerCode = body.error?.code ?? body.error?.type
    const providerRequestId = error.headers.get('x-request-id')
      ?? body.request_id
    return withRetryWait({
      type: 'error',
      message: error.message,
      code: httpErrorCode(error.status, error.message),
      diagnostics: {
        source: 'http',
        httpStatus: error.status,
        ...(providerCode ? { providerCode } : {}),
        ...(providerRequestId ? { providerRequestId } : {}),
        upstream: httpFailureRecord(error.status, error.headers, error.responseText),
      },
    }, readCodexFailure)
  }
  const message = error instanceof Error ? error.message : String(error)
  return {
    type: 'error',
    message,
    code: normalizeErrorCode(null, message),
    diagnostics: { source: 'transport' },
  }
}

function defaultUserAgent(): string {
  return `demi-codex-provider/0.0.0 (${process.platform}; ${process.arch})`
}
