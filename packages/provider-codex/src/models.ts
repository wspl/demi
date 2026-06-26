import { errorMessage, isRecord, nonEmptyString, numberOrNull } from '@demi/utils'
import type { ProviderModel, ProviderModelList } from '@demi/provider'
import {
  CodexAuthError,
  FileCodexAuthStore,
  redactSecretText,
  type CodexAuthStore,
  type CodexResolvedAuth,
} from './auth'

export interface CodexModelCatalogOptions {
  authStore?: CodexAuthStore
  codexHome?: string
  baseUrl?: string
  headers?: Record<string, string>
  userAgent?: string
  clientVersion?: string
  fetch?: ModelCatalogFetch
  now?: () => Date
}

export type ModelCatalogFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

interface CodexCatalogCache {
  fetchedAtMs: number
  list: ProviderModelList
}

const DEFAULT_CHATGPT_CODEX_BASE_URL = 'https://chatgpt.com/backend-api'
const DEFAULT_CODEX_MODEL_CATALOG_CLIENT_VERSION = '0.130.0'
const CODEX_MODEL_CACHE_TTL_MS = 15 * 60 * 1000
const codexCatalogCache = new Map<string, CodexCatalogCache>()

export async function listCodexModels(options: CodexModelCatalogOptions = {}): Promise<ProviderModelList> {
  const fetchImpl = options.fetch ?? fetch
  const now = options.now ?? (() => new Date())
  const nowDate = now()
  const authStore = options.authStore ?? new FileCodexAuthStore({ codexHome: options.codexHome })
  const clientVersion = options.clientVersion ?? DEFAULT_CODEX_MODEL_CATALOG_CLIENT_VERSION
  const auth = await authStore.resolveAuth()
  assertCodexBackendModelCatalogAuth(auth)
  const cacheKey = codexModelCatalogCacheKey(auth, options.baseUrl, clientVersion)
  const cached = codexCatalogCache.get(cacheKey)
  if (cached && nowDate.getTime() - cached.fetchedAtMs < CODEX_MODEL_CACHE_TTL_MS) {
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
    })
    codexCatalogCache.set(cacheKey, { fetchedAtMs: nowDate.getTime(), list })
    return cloneModelList(list)
  } catch (error) {
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
      })
      codexCatalogCache.set(codexModelCatalogCacheKey(refreshed, options.baseUrl, clientVersion), {
        fetchedAtMs: nowDate.getTime(),
        list,
      })
      return cloneModelList(list)
    }
    if (cached && !isAuthCatalogError(error)) {
      const stale = markModelListCache(cached.list, true)
      return {
        ...cloneModelList(stale),
        warnings: [...stale.warnings, `Using stale Codex model catalog: ${errorMessage(error)}`],
      }
    }
    throw error
  }
}

export function codexBackendModelsToModelList(
  value: unknown,
  options: {
    sourceFetchedAt?: string
    stale?: boolean
    warnings?: string[]
  } = {},
): ProviderModelList {
  if (!isRecord(value) || !Array.isArray(value.models)) {
    throw new Error('Codex models response does not contain a models array')
  }
  const sourceFetchedAt = options.sourceFetchedAt ?? new Date().toISOString()
  const warnings = [...(options.warnings ?? [])]
  const models: ProviderModel[] = []
  for (const raw of value.models) {
    if (!isRecord(raw)) {
      warnings.push('Skipped Codex model with invalid metadata')
      continue
    }
    const id = nonEmptyString(raw.slug)
    if (!id) {
      warnings.push('Skipped Codex model without slug')
      continue
    }
    if (nonEmptyString(raw.visibility) === 'hide') continue
    models.push(codexModelFromBackendEntry(id, raw, sourceFetchedAt, options.stale === true))
  }
  return {
    providerId: 'codex',
    models,
    defaultModelId: null,
    warnings,
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
}): Promise<ProviderModelList> {
  const response = await options.fetch(codexModelsUrl(options.baseUrl ?? DEFAULT_CHATGPT_CODEX_BASE_URL, options.clientVersion), {
    headers: buildCodexModelCatalogHeaders(options.auth, options.headers, options.userAgent),
  })
  if (!response.ok) {
    throw new CodexModelCatalogHttpError(response.status, `Codex models request failed with HTTP ${response.status}`)
  }
  return codexBackendModelsToModelList(await response.json(), {
    sourceFetchedAt: options.now.toISOString(),
    stale: false,
  })
}

function codexModelFromBackendEntry(
  id: string,
  raw: Record<string, unknown>,
  sourceFetchedAt: string,
  stale: boolean,
): ProviderModel {
  const supportedReasoningEfforts = supportedReasoningLevels(raw.supported_reasoning_levels)
  const tiers = serviceTiers(raw.service_tiers)
  return {
    providerId: 'codex',
    id,
    displayName: nonEmptyString(raw.display_name) ?? id,
    description: nonEmptyString(raw.description),
    contextWindow: numberOrNull(raw.context_window),
    outputLimit: null,
    supportsTools: supportsCodexTools(raw),
    supportsAttachments: Array.isArray(raw.input_modalities) ? raw.input_modalities.includes('image') : null,
    supportsReasoning: supportedReasoningEfforts ? supportedReasoningEfforts.length > 0 : null,
    supportedThinkingEfforts: supportedReasoningEfforts,
    defaultThinkingEffort: null,
    serviceTiers: tiers,
    defaultServiceTierId: null,
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
  if (auth.kind === 'agentIdentity') headers.set('Authorization', auth.authorization)
  else headers.set('Authorization', `Bearer ${auth.accessToken}`)
  if (auth.accountId) headers.set('ChatGPT-Account-ID', auth.accountId)
  if ('isFedrampAccount' in auth && auth.isFedrampAccount) headers.set('X-OpenAI-Fedramp', 'true')
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

function assertCodexBackendModelCatalogAuth(auth: CodexResolvedAuth): asserts auth is Exclude<CodexResolvedAuth, { kind: 'apiKey' }> {
  if (auth.kind === 'apiKey') {
    throw new CodexAuthError('auth_unsupported', 'Codex backend model catalog requires official Codex ChatGPT auth, not OPENAI_API_KEY')
  }
}

function codexModelCatalogCacheKey(auth: CodexResolvedAuth, baseUrl: string | undefined, clientVersion: string): string {
  const account = 'accountId' in auth ? auth.accountId ?? '' : ''
  return [auth.kind, auth.mode, account, baseUrl ?? DEFAULT_CHATGPT_CODEX_BASE_URL, clientVersion].join('\0')
}

function supportedReasoningLevels(value: unknown): ProviderModel['supportedThinkingEfforts'] {
  if (!Array.isArray(value)) return null
  const efforts = value
    .map((level) => isRecord(level) ? nonEmptyString(level.effort) : undefined)
    .filter((effort): effort is string => effort !== undefined)
  return efforts.length > 0 ? efforts : []
}

function serviceTiers(value: unknown): ProviderModel['serviceTiers'] {
  if (!Array.isArray(value)) return null
  const tiers = value.flatMap((tier) => {
    if (!isRecord(tier)) return []
    const id = nonEmptyString(tier.id)
    if (!id) return []
    return [{
      id,
      label: nonEmptyString(tier.name) ?? id,
      ...(nonEmptyString(tier.description) ? { description: nonEmptyString(tier.description) } : {}),
    }]
  })
  return tiers.length > 0 ? tiers : []
}

function supportsCodexTools(raw: Record<string, unknown>): boolean | null {
  if (typeof raw.tool_mode === 'string' && raw.tool_mode.length > 0) return true
  if (Array.isArray(raw.experimental_supported_tools)) return raw.experimental_supported_tools.length > 0
  if (raw.apply_patch_tool_type !== undefined || raw.web_search_tool_type !== undefined) return true
  return null
}

function markModelListCache(list: ProviderModelList, stale: boolean): ProviderModelList {
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
      supportedThinkingEfforts: model.supportedThinkingEfforts ? [...model.supportedThinkingEfforts] : null,
      serviceTiers: model.serviceTiers ? model.serviceTiers.map((tier) => ({ ...tier })) : model.serviceTiers,
    })),
  }
}

function isUnauthorized(error: unknown): boolean {
  return error instanceof CodexModelCatalogHttpError && error.status === 401
}

function isAuthCatalogError(error: unknown): boolean {
  if (error instanceof CodexModelCatalogHttpError && (error.status === 401 || error.status === 403)) return true
  return error instanceof CodexAuthError
}

class CodexModelCatalogHttpError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(redactSecretText(message))
    this.name = 'CodexModelCatalogHttpError'
  }
}

function defaultModelCatalogUserAgent(): string {
  return `demi-provider-codex/0.0.0`
}

