import { isAbortError, normalizeBaseUrl } from '@demicodes/utils'
import { z } from 'zod'
import {
  defineProvider,
  httpRequestFailedEvent,
  mapChatCompletionsStream,
  providerErrorFromUnknown,
  type ProviderQuotaSnapshots,
  readHttpFailure,
  readServerSentEvents,
  type AgentProvider,
  type InferenceRequest,
  type Provider,
  type ProviderEvent,
  type ProviderQuota,
  type ProviderSelection,
} from '@demicodes/provider'
import {
  GrokAuthError,
  type GrokAuthStore,
  type GrokResolvedAuth,
} from './auth'
import { buildGrokChatCompletionsBody } from './chat'
import {
  createGrokBuildCredentials,
  openGrokCredentialPool,
  PoolAwareGrokAuthStore
} from './credentials'
import { DEFAULT_GROK_BUILD_BASE_URL, buildGrokBuildHeaders } from './headers'
import { listGrokBuildModels } from './models'
import {
  fallbackCredentialPool,
  fileQuotaSnapshots,
  quotaSnapshotFile,
  type CredentialPool,
} from '@demicodes/provider/credentials-pool'
import { createGrokBuildQuota } from './quota'
import { grokVendorPool } from './vendor'

const GROK_VENDOR_LABEL = 'Grok Build'

export type GrokBuildFetch = (
  input: string | URL | Request,
  init?: RequestInit
) => Promise<Response>

/**
 * The Grok Build provider's configuration, as a config file states it. Unknown
 * keys are rejected: a misspelled key would otherwise be silently ignored.
 */
export const grokBuildProviderConfigSchema = z.strictObject({
  grokHome: z.string().optional(),
  baseUrl: z.string().optional(),
  headers: z.record(z.string(), z.string()).optional(),
})

export type GrokBuildProviderConfig =
  z.infer<typeof grokBuildProviderConfigSchema>

export interface GrokBuildProviderOptions {
  id?: string
  displayName?: string
  grokHome?: string
  baseUrl?: string
  clientVersion?: string
  authStore?: GrokAuthStore
  headers?: Record<string, string>
  fetch?: GrokBuildFetch
  /** Demi state root for credential pool (`$DEMI_HOME` / `~/.demi`). */
  stateDir?: string
  /** The accounts of this entry, kept by the caller instead of under `stateDir`. */
  credentialPool?: CredentialPool
  /** The account this provider stands for, instead of the pool's active one. */
  credentialId?: string
  /** Keeps the account's usage snapshot; by default a file in `stateDir`. */
  quotaSnapshots?: ProviderQuotaSnapshots
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
        yield providerErrorFromUnknown(error)
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
            GROK_VENDOR_LABEL,
            readHttpFailure
          )
          return
        }

        yield* mapChatCompletionsStream(
          readServerSentEvents(response.body, request.cancel),
          GROK_VENDOR_LABEL,
          readHttpFailure,
          request.cancel,
        )
        return
      } catch (error) {
        if (request.cancel.aborted || isAbortError(error)) {
          yield { type: 'abort' }
          return
        }
        yield providerErrorFromUnknown(error)
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
  // A caller that passes no pool gets Demi's files, standing for the Grok
  // CLI's own logins while they hold no account.
  const vendor = grokVendorPool({ grokHome: options.grokHome })
  const pool = options.credentialPool ?? (enableCredentials
    ? fallbackCredentialPool(
      openGrokCredentialPool({ stateDir: options.stateDir }),
      vendor
    )
    : vendor)
  const authStore: GrokAuthStore = options.authStore
    ?? new PoolAwareGrokAuthStore(pool, { credentialId: options.credentialId })
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
    snapshots: options.quotaSnapshots
      ?? fileQuotaSnapshots(quotaSnapshotFile(options.stateDir), id),
  })
  const credentialsApi = !options.authStore && enableCredentials
    ? createGrokBuildCredentials(pool, authStore, {
      ...(options.credentialPool ? {} : { importFrom: vendor }),
      quota,
      pinned: options.credentialId !== undefined,
    })
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
    readFailure: readHttpFailure,
    auth: { status: () => authStore.status() },
    quota,
    ...(credentialsApi ? { credentials: credentialsApi } : {}),
    state: () => ({
      status: 'ready',
      message: credentialsApi
        ? 'Uses Grok CLI OAuth + demi credential pool via cli-chat-proxy'
        : 'Uses Grok CLI OAuth session (~/.grok/auth.json) via cli-chat-proxy',
    }),
    listModels: (listOptions) =>
      listGrokBuildModels({
        signal: listOptions?.signal,
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

export function parseGrokBuildProviderConfig(
  config: unknown
): GrokBuildProviderConfig {
  return grokBuildProviderConfigSchema.parse(config ?? {})
}

function chatCompletionsUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/chat/completions')
    ? normalized
    : `${normalized}/chat/completions`
}
