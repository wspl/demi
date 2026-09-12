import { readServerSentEvents } from '@demicodes/provider'
import { grokBuildConfigSchema, type GrokBuildProviderConfig } from './config-schema'
import { isAbortError, normalizeBaseUrl } from '@demicodes/utils'
import {
  defineProvider,
  parseProviderData,
  httpRequestFailedEvent,
  providerErrorFromUnknown,
  type AgentProvider,
  type InferenceRequest,
  type Provider,
  type ProviderEvent,
  type ProviderQuota,
  type ProviderSelection,
} from '@demicodes/provider'
import {
  FileGrokAuthStore,
  GrokAuthError,
  type GrokAuthStore,
  type GrokResolvedAuth,
} from './auth'
import {
  buildGrokChatCompletionsBody,
  mapGrokChatCompletionStream,
} from './chat'
import {
  createGrokBuildCredentials,
  openGrokCredentialPool,
  PoolAwareGrokAuthStore
} from './credentials'
import { DEFAULT_GROK_BUILD_BASE_URL, buildGrokBuildHeaders } from './headers'
import { listGrokBuildModels } from './models'
import { createGrokBuildQuota } from './quota'

export type GrokBuildFetch = (
  input: string | URL | Request,
  init?: RequestInit
) => Promise<Response>

export interface GrokBuildProviderOptions extends GrokBuildProviderConfig {
  id?: string
  displayName?: string
  clientVersion?: string
  authStore?: GrokAuthStore
  fetch?: GrokBuildFetch
  /** Demi state root for credential pool (`$DEMI_HOME` / `~/.demi`). */
  stateDir?: string
  /** When true (default if `authStore` unset), attach multi-credential pool. */
  credentials?: boolean
}

interface GrokBuildRuntimeOptions {
  baseUrl: string
  grokHome?: string
  clientVersion?: string
  authStore: GrokAuthStore
  headers?: Record<string, string>
  fetch: GrokBuildFetch
  quota?: ProviderQuota
}

export class GrokBuildHttpError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message)
    this.name = 'GrokBuildHttpError'
  }
}

export class GrokBuildProvider implements AgentProvider {
  constructor(private readonly options: GrokBuildRuntimeOptions) {}

  clone(): AgentProvider {
    return new GrokBuildProvider(this.options)
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (request.cancel.aborted) {
      yield { type: 'abort' }
      return
    }

    let forceRefresh = false
    let accessToken: string | undefined

    for (let attempt = 0; attempt < 2; attempt++) {
      const observeQuota = this.options.quota?.captureObserver()
      let auth: GrokResolvedAuth
      try {
        auth = await this.options.authStore.resolveAuth({ forceRefresh })
        accessToken = auth.accessToken
      } catch (error) {
        if (error instanceof GrokAuthError && error.code === 'auth_missing') {
          yield { type: 'error', message: error.message, code: 'auth_missing' }
          return
        }
        yield providerErrorFromUnknown(error, accessToken)
        return
      }

      try {
        const headers = buildGrokBuildHeaders(auth, request, {
          extra: this.options.headers,
          clientVersion: this.options.clientVersion,
          grokHome: this.options.grokHome,
        })
        headers.set('accept', 'text/event-stream')
        headers.set('content-type', 'application/json')

        const response = await this.options.fetch(
          chatCompletionsUrl(this.options.baseUrl),
          {
            method: 'POST',
            headers,
            body: JSON.stringify(buildGrokChatCompletionsBody(request)),
            signal: request.cancel,
          }
        )

        try {
          observeQuota?.({ headers: response.headers, status: response.status })
        } catch {
          // Quota observation must never break inference.
        }

        if (response.status === 401 && !forceRefresh) {
          await response.body?.cancel().catch(() => {})
          forceRefresh = true
          continue
        }
        if (!response.ok) {
          yield await httpRequestFailedEvent(
            response,
            accessToken,
            'Grok Build'
          )
          return
        }

        yield* mapGrokChatCompletionStream(
          readServerSentEvents(response.body, request.cancel),
          request.cancel
        )
        return
      } catch (error) {
        if (request.cancel.aborted || isAbortError(error)) {
          yield { type: 'abort' }
          return
        }
        yield providerErrorFromUnknown(error, accessToken)
        return
      }
    }
  }
}

export function createGrokBuildProvider(
  options: GrokBuildProviderOptions = {}
): Provider {
  const id = options.id ?? 'grok-build'
  const displayName = options.displayName ?? 'Grok Build'
  const enableCredentials = options.credentials
    ?? options.authStore === undefined
  const pool = !options.authStore && enableCredentials ? openGrokCredentialPool(
    {
      stateDir: options.stateDir
    }
  ) : null
  const authStore =
    options.authStore ??
    (pool
      ? new PoolAwareGrokAuthStore(pool, { grokHome: options.grokHome })
      : new FileGrokAuthStore({ grokHome: options.grokHome }))
  const baseUrl = normalizeBaseUrl(options.baseUrl
    ?? DEFAULT_GROK_BUILD_BASE_URL)
  const fetchImpl = options.fetch ?? fetch
  const quota = createGrokBuildQuota({
    providerId: id,
    grokHome: options.grokHome,
    baseUrl,
    clientVersion: options.clientVersion,
    authStore,
    fetch: fetchImpl as GrokBuildFetch,
  })
  const credentialsApi = pool
    ? createGrokBuildCredentials(
      pool,
      authStore,
      { grokHome: options.grokHome, quota }
    )
    : undefined
  const runtimeOptions: GrokBuildRuntimeOptions = {
    baseUrl,
    grokHome: options.grokHome,
    clientVersion: options.clientVersion,
    authStore,
    headers: options.headers,
    fetch: fetchImpl,
    quota,
  }

  return defineProvider({
    id,
    displayName,
    auth: { status: () => authStore.status() },
    quota,
    ...(credentialsApi ? { credentials: credentialsApi } : {}),
    state: () => ({
      status: 'ready',
      message: credentialsApi
        ? 'Uses Grok CLI OAuth + demi credential pool via cli-chat-proxy'
        : 'Uses Grok CLI OAuth session (~/.grok/auth.json) via cli-chat-proxy',
    }),
    listModels: () =>
      listGrokBuildModels({
        providerId: id,
        grokHome: options.grokHome,
        baseUrl,
        clientVersion: options.clientVersion,
        authStore,
        fetch: fetchImpl,
      }),
    createRuntime: (_selection: ProviderSelection) => new GrokBuildProvider(runtimeOptions),
  })
}

export function parseGrokBuildProviderConfig(config: unknown): GrokBuildProviderConfig {
  return parseProviderData(grokBuildConfigSchema, config ?? {}, 'Grok Build provider config')
}

function chatCompletionsUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/chat/completions')
    ? normalized
    : `${normalized}/chat/completions`
}
