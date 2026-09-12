import { z } from 'zod'
import { errorMessage } from '@demicodes/utils'
import type {
  ProviderModel,
  ProviderModelList,
  ProviderModelListOptions
} from '@demicodes/provider'
import {
  CodexAuthError,
  FileCodexAuthStore,
  redactCodexSecretText,
  type CodexAuthStore,
  type CodexResolvedAuth,
} from './auth'

export interface CodexModelCatalogOptions extends ProviderModelListOptions {
  authStore?: CodexAuthStore
  codexHome?: string
  baseUrl?: string
  headers?: Record<string, string>
  userAgent?: string
  clientVersion?: string
  fetch?: ModelCatalogFetch
  now?: () => Date
}

export type ModelCatalogFetch = (
  input: string | URL | Request,
  init?: RequestInit
) => Promise<Response>

interface CodexCatalogCache {
  fetchedAtMs: number
  list: ProviderModelList
}

const DEFAULT_CHATGPT_CODEX_BASE_URL = 'https://chatgpt.com/backend-api'
// Match a verified Codex CLI release: the backend gates models by this version.
const DEFAULT_CODEX_MODEL_CATALOG_CLIENT_VERSION = '0.153.4'
const CODEX_MODEL_CACHE_TTL_MS = 15 * 60 * 1000
const codexCatalogCache = new Map<string, CodexCatalogCache>()

export async function listCodexModels(
  options: CodexModelCatalogOptions = {}
): Promise<ProviderModelList> {
  const fetchImpl = options.fetch ?? fetch
  const now = options.now ?? (() => new Date())
  const nowDate = now()
  const authStore = options.authStore ?? new FileCodexAuthStore({
    codexHome: options.codexHome
  })
  const clientVersion = options.clientVersion
    ?? DEFAULT_CODEX_MODEL_CATALOG_CLIENT_VERSION
  const auth = await authStore.resolveAuth()
  assertCodexBackendModelCatalogAuth(auth)
  const cacheKey = codexModelCatalogCacheKey(
    auth,
    options.baseUrl,
    clientVersion
  )
  const cached = codexCatalogCache.get(cacheKey)
  if (!options.refresh && cached
    && nowDate.getTime() - cached.fetchedAtMs < CODEX_MODEL_CACHE_TTL_MS) {
    return cloneModelList(markModelListCache(cached.list, false))
  }

  try {
    const list = await requestCodexModels({
      auth,
      clientVersion,
      fetch: fetchImpl,
      now: nowDate,
      baseUrl: options.baseUrl,
      headers: options.headers,
      userAgent: options.userAgent,
      signal: options.signal,
    })
    codexCatalogCache.set(cacheKey, { fetchedAtMs: nowDate.getTime(), list })
    return cloneModelList(list)
  } catch (error) {
    options.signal?.throwIfAborted()
    if (isUnauthorized(error)) {
      const refreshed = await authStore.resolveAuth({ forceRefresh: true })
      assertCodexBackendModelCatalogAuth(refreshed)
      const list = await requestCodexModels({
        auth: refreshed,
        clientVersion,
        fetch: fetchImpl,
        now: nowDate,
        baseUrl: options.baseUrl,
        headers: options.headers,
        userAgent: options.userAgent,
        signal: options.signal,
      })
      codexCatalogCache.set(codexModelCatalogCacheKey(
        refreshed,
        options.baseUrl,
        clientVersion
      ), {
        fetchedAtMs: nowDate.getTime(),
        list,
      })
      return cloneModelList(list)
    }
    if (cached && !isAuthCatalogError(error)) {
      const stale = markModelListCache(cached.list, true)
      return {
        ...cloneModelList(stale),
        warnings: [
          ...stale.warnings,
          `Using stale Codex model catalog: ${errorMessage(error)}`
        ],
      }
    }
    throw error
  }
}

const codexModelSchema = z.object({
  slug: z.string().min(1),
  display_name: z.string().min(1),
  description: z.string().nullish(),
  visibility: z.enum(['list', 'hide', 'none']),
  priority: z.number().int(),
  context_window: z.number().int().positive().nullish(),
  input_modalities: z.array(z.string()).optional(),
  supported_reasoning_levels: z.array(z.object({ effort: z.string().min(1) })),
  default_reasoning_level: z.string().min(1).nullish(),
  service_tiers: z.array(z.object({
    id: z.string().min(1),
    name: z.string().min(1),
    description: z.string().nullish(),
  })).optional(),
  default_service_tier: z.string().min(1).nullish(),
  tool_mode: z.string().nullish(),
  experimental_supported_tools: z.array(z.string()).optional(),
  apply_patch_tool_type: z.string().nullish(),
  web_search_tool_type: z.string().nullish(),
})
const codexModelsResponseSchema = z.object({ models: z.array(codexModelSchema) })
type CodexBackendModel = z.infer<typeof codexModelSchema>

export function codexBackendModelsToModelList(
  value: unknown,
  options: {
    sourceFetchedAt?: string
    stale?: boolean
    warnings?: string[]
  } = {},
): ProviderModelList {
  const response = codexModelsResponseSchema.parse(value)
  const sourceFetchedAt = options.sourceFetchedAt ?? new Date().toISOString()
  const models = response.models
    .filter((model) => model.visibility === 'list')
    .sort((a, b) => a.priority - b.priority)
    .map((model) => codexModelFromBackendEntry(
      model,
      sourceFetchedAt,
      options.stale === true,
    ))
  return {
    providerId: 'codex',
    models,
    defaultModelId: models[0]?.id ?? null,
    warnings: [...(options.warnings ?? [])],
    sourceFetchedAt,
    stale: options.stale === true,
  }
}

export function resetCodexModelCatalogCacheForTests(): void {
  codexCatalogCache.clear()
}

async function requestCodexModels(options: {
  auth: Exclude<CodexResolvedAuth, { kind: 'apiKey' }>
  clientVersion: string
  fetch: ModelCatalogFetch
  now: Date
  baseUrl?: string
  headers?: Record<string, string>
  userAgent?: string
  signal?: AbortSignal
}): Promise<ProviderModelList> {
  const response = await options.fetch(codexModelsUrl(
    options.baseUrl ?? DEFAULT_CHATGPT_CODEX_BASE_URL,
    options.clientVersion
  ), {
    signal: options.signal,
    headers: buildCodexModelCatalogHeaders(
      options.auth,
      options.headers,
      options.userAgent
    ),
  })
  if (!response.ok) {
    throw new CodexModelCatalogHttpError(
      response.status,
      `Codex models request failed with HTTP ${response.status}`
    )
  }
  return codexBackendModelsToModelList(await response.json(), {
    sourceFetchedAt: options.now.toISOString(),
    stale: false,
  })
}

function codexModelFromBackendEntry(
  raw: CodexBackendModel,
  sourceFetchedAt: string,
  stale: boolean,
): ProviderModel {
  const supportedReasoningEfforts = raw.supported_reasoning_levels.map(
    (level) => level.effort,
  )
  return {
    providerId: 'codex',
    id: raw.slug,
    displayName: raw.display_name,
    description: raw.description ?? undefined,
    contextWindow: raw.context_window ?? null,
    outputLimit: null,
    supportsTools: supportsCodexTools(raw),
    supportsAttachments: raw.input_modalities?.includes('image') ?? null,
    supportsReasoning: supportedReasoningEfforts.length > 0,
    // Omitting reasoning uses the backend default; it does not turn reasoning off.
    canDisableThinking: false,
    supportedThinkingEfforts: supportedReasoningEfforts,
    defaultThinkingEffort: raw.default_reasoning_level ?? null,
    serviceTiers: raw.service_tiers?.map((tier) => ({
      id: tier.id,
      label: tier.name,
      ...(tier.description ? { description: tier.description } : {}),
      fast: tier.id === CODEX_FAST_SERVICE_TIER_ID,
    })) ?? null,
    defaultServiceTierId: raw.default_service_tier ?? null,
    sourceFetchedAt,
    stale,
  }
}

function buildCodexModelCatalogHeaders(
  auth: Exclude<CodexResolvedAuth, { kind: 'apiKey' }>,
  configuredHeaders: Record<string, string> | undefined,
  userAgent: string | undefined,
): Headers {
  const headers = new Headers(configuredHeaders)
  if (auth.kind === 'agentIdentity') headers.set(
    'Authorization',
    auth.authorization
  )
  else headers.set('Authorization', `Bearer ${auth.accessToken}`)
  if (auth.accountId)
    headers.set('ChatGPT-Account-ID', auth.accountId)
  if ('isFedrampAccount' in auth && auth.isFedrampAccount)
    headers.set(
      'X-OpenAI-Fedramp',
      'true'
    )
  headers.set('accept', 'application/json')
  headers.set('User-Agent', userAgent ?? defaultModelCatalogUserAgent())
  return headers
}

function codexModelsUrl(baseUrl: string, clientVersion: string): string {
  const normalized = baseUrl.replace(/\/+$/, '')
  const path = normalized.endsWith('/codex/models')
    ? normalized
    : normalized.endsWith('/codex')
      ? `${normalized}/models`
      : `${normalized}/codex/models`
  return `${path}?client_version=${encodeURIComponent(clientVersion)}`
}

function assertCodexBackendModelCatalogAuth(
  auth: CodexResolvedAuth
): asserts auth is Exclude<CodexResolvedAuth, { kind: 'apiKey' }> {
  if (auth.kind === 'apiKey') {
    throw new CodexAuthError(
      'auth_unsupported',
      'Codex backend model catalog requires official Codex ChatGPT auth, not OPENAI_API_KEY'
    )
  }
}

function codexModelCatalogCacheKey(
  auth: CodexResolvedAuth,
  baseUrl: string | undefined,
  clientVersion: string
): string {
  const account = 'accountId' in auth ? auth.accountId ?? '' : ''
  return [
    auth.kind,
    auth.mode,
    account,
    baseUrl ?? DEFAULT_CHATGPT_CODEX_BASE_URL,
    clientVersion
  ].join('\0')
}

/** Codex advertises its Fast Mode as the `priority` service tier. */
const CODEX_FAST_SERVICE_TIER_ID = 'priority'

function supportsCodexTools(raw: CodexBackendModel): boolean | null {
  if (typeof raw.tool_mode === 'string' && raw.tool_mode.length > 0)
    return true
  if (Array.isArray(raw.experimental_supported_tools))
    return raw.experimental_supported_tools.length > 0
  if (raw.apply_patch_tool_type !== undefined
    || raw.web_search_tool_type !== undefined) return true
  return null
}

function markModelListCache(
  list: ProviderModelList,
  stale: boolean
): ProviderModelList {
  return {
    ...list,
    stale,
    models: list.models.map((model) => ({ ...model, stale })),
  }
}

function cloneModelList(list: ProviderModelList): ProviderModelList {
  return {
    ...list,
    warnings: [...list.warnings],
    models: list.models.map((model) => ({
      ...model,
      ...(model.cost ? { cost: { ...model.cost } } : {}),
      supportedThinkingEfforts: model.supportedThinkingEfforts
        ? [...model.supportedThinkingEfforts]
        : null,
      serviceTiers: model.serviceTiers ? model.serviceTiers.map((tier) => ({
        ...tier
      })) : model.serviceTiers,
    })),
  }
}

function isUnauthorized(error: unknown): boolean {
  return error instanceof CodexModelCatalogHttpError && error.status === 401
}

function isAuthCatalogError(error: unknown): boolean {
  if (error instanceof CodexModelCatalogHttpError
    && (error.status === 401 || error.status === 403)) return true
  return error instanceof CodexAuthError
}

class CodexModelCatalogHttpError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(redactCodexSecretText(message))
    this.name = 'CodexModelCatalogHttpError'
  }
}

function defaultModelCatalogUserAgent(): string {
  return `demi-provider-codex/0.0.0`
}

