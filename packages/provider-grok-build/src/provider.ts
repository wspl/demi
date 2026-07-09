import { isAbortError, isRecord, normalizeBaseUrl } from '@demicodes/utils'
import {
  defineProvider,
  httpRequestFailedEvent,
  providerErrorFromUnknown,
  type AgentProvider,
  type InferenceRequest,
  type Provider,
  type ProviderEvent,
  type ProviderSelection,
} from '@demicodes/provider'
import {
  FileGrokAuthStore,
  GrokAuthError,
  type GrokAuthStore,
  type GrokResolvedAuth,
} from './auth'
import { buildGrokChatCompletionsBody, mapGrokChatCompletionStream, readServerSentEvents } from './chat'
import { DEFAULT_GROK_BUILD_BASE_URL, buildGrokBuildHeaders } from './headers'
import { listGrokBuildModels } from './models'

export type GrokBuildFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

export interface GrokBuildProviderOptions {
  id?: string
  displayName?: string
  grokHome?: string
  baseUrl?: string
  clientVersion?: string
  authStore?: GrokAuthStore
  headers?: Record<string, string>
  fetch?: GrokBuildFetch
}

interface GrokBuildRuntimeOptions {
  baseUrl: string
  grokHome?: string
  clientVersion?: string
  authStore: GrokAuthStore
  headers?: Record<string, string>
  fetch: GrokBuildFetch
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

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (request.cancel.aborted) {
      yield { type: 'abort' }
      return
    }

    let forceRefresh = false
    let accessToken: string | undefined

    for (let attempt = 0; attempt < 2; attempt++) {
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

        const response = await this.options.fetch(chatCompletionsUrl(this.options.baseUrl), {
          method: 'POST',
          headers,
          body: JSON.stringify(buildGrokChatCompletionsBody(request)),
          signal: request.cancel,
        })

        if (response.status === 401 && !forceRefresh) {
          forceRefresh = true
          continue
        }
        if (!response.ok) {
          yield await httpRequestFailedEvent(response, accessToken, 'Grok Build')
          return
        }

        yield* mapGrokChatCompletionStream(readServerSentEvents(response.body, request.cancel), request.cancel)
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

export function createGrokBuildProvider(options: GrokBuildProviderOptions = {}): Provider {
  const id = options.id ?? 'grok-build'
  const displayName = options.displayName ?? 'Grok Build'
  const authStore = options.authStore ?? new FileGrokAuthStore({ grokHome: options.grokHome })
  const baseUrl = normalizeBaseUrl(options.baseUrl ?? DEFAULT_GROK_BUILD_BASE_URL)
  const fetchImpl = options.fetch ?? fetch
  const runtimeOptions: GrokBuildRuntimeOptions = {
    baseUrl,
    grokHome: options.grokHome,
    clientVersion: options.clientVersion,
    authStore,
    headers: options.headers,
    fetch: fetchImpl,
  }

  return defineProvider({
    id,
    displayName,
    auth: { status: () => authStore.status() },
    state: () => ({
      status: 'ready',
      message: 'Uses Grok CLI OAuth session (~/.grok/auth.json) via cli-chat-proxy',
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

export function parseGrokBuildProviderConfig(config: unknown): Pick<GrokBuildProviderOptions, 'grokHome' | 'baseUrl' | 'headers'> {
  if (config === undefined || config === null) return {}
  if (!isRecord(config)) throw new Error('Grok Build provider config must be an object')
  const parsed: Pick<GrokBuildProviderOptions, 'grokHome' | 'baseUrl' | 'headers'> = {}
  if (config.grokHome !== undefined) {
    if (typeof config.grokHome !== 'string') throw new Error('Grok Build provider config field "grokHome" must be a string')
    parsed.grokHome = config.grokHome
  }
  if (config.baseUrl !== undefined) {
    if (typeof config.baseUrl !== 'string') throw new Error('Grok Build provider config field "baseUrl" must be a string')
    parsed.baseUrl = config.baseUrl
  }
  if (config.headers !== undefined) {
    if (!isRecord(config.headers)) throw new Error('Grok Build provider config field "headers" must be an object')
    const headers: Record<string, string> = {}
    for (const [key, value] of Object.entries(config.headers)) {
      if (typeof value !== 'string') throw new Error(`Grok Build provider config headers.${key} must be a string`)
      headers[key] = value
    }
    parsed.headers = headers
  }
  return parsed
}

function chatCompletionsUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/chat/completions') ? normalized : `${normalized}/chat/completions`
}
