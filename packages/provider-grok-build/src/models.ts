import { isRecord, nonEmptyString, normalizeBaseUrl, numberOrNull, stringOrNull } from '@demicodes/utils'
import type { ProviderModel, ProviderModelList } from '@demicodes/provider'
import type { GrokAuthStore } from './auth'
import { FileGrokAuthStore } from './auth'
import { DEFAULT_GROK_BUILD_BASE_URL, buildGrokBuildHeaders } from './headers'

export interface GrokBuildModelCatalogOptions {
  providerId?: string
  grokHome?: string
  baseUrl?: string
  clientVersion?: string
  authStore?: GrokAuthStore
  fetch?: (input: string | URL | Request, init?: RequestInit) => Promise<Response>
}

const FALLBACK_SOURCE_FETCHED_AT = '1970-01-01T00:00:00.000Z'

export function grokBuildFallbackModels(providerId = 'grok-build'): ProviderModelList {
  const sourceFetchedAt = FALLBACK_SOURCE_FETCHED_AT
  const model: ProviderModel = {
    providerId,
    id: 'grok-4.5',
    displayName: 'Grok 4.5',
    description: 'Grok Build frontier model',
    contextWindow: 500_000,
    outputLimit: null,
    supportsTools: true,
    supportsAttachments: true,
    supportsReasoning: true,
    supportedThinkingEfforts: ['low', 'medium', 'high'],
    defaultThinkingEffort: 'high',
    canDisableThinking: null,
    serviceTiers: null,
    defaultServiceTierId: null,
    sourceFetchedAt,
    stale: true,
  }
  return {
    providerId,
    models: [model],
    defaultModelId: model.id,
    warnings: ['Using fallback Grok Build model catalog (live /v1/models unavailable)'],
    sourceFetchedAt,
    stale: true,
  }
}

export async function listGrokBuildModels(options: GrokBuildModelCatalogOptions = {}): Promise<ProviderModelList> {
  const providerId = options.providerId ?? 'grok-build'
  const authStore = options.authStore ?? new FileGrokAuthStore({ grokHome: options.grokHome })
  const fetchImpl = options.fetch ?? fetch
  const baseUrl = normalizeBaseUrl(options.baseUrl ?? DEFAULT_GROK_BUILD_BASE_URL)

  try {
    const auth = await authStore.resolveAuth()
    const response = await fetchImpl(modelsUrl(baseUrl), {
      method: 'GET',
      headers: buildGrokBuildHeaders(auth, undefined, {
        clientVersion: options.clientVersion,
        grokHome: options.grokHome,
      }),
    })
    if (!response.ok) {
      const fallback = grokBuildFallbackModels(providerId)
      return {
        ...fallback,
        warnings: [`Grok Build /v1/models returned HTTP ${response.status}; using fallback catalog`],
      }
    }
    const payload = (await response.json()) as unknown
    return modelListFromGrokModelsPayload(payload, providerId)
  } catch {
    return grokBuildFallbackModels(providerId)
  }
}

export function modelListFromGrokModelsPayload(payload: unknown, providerId: string): ProviderModelList {
  const sourceFetchedAt = new Date().toISOString()
  const data = isRecord(payload) && Array.isArray(payload.data) ? payload.data : Array.isArray(payload) ? payload : []
  const models: ProviderModel[] = []

  for (const item of data) {
    if (!isRecord(item)) continue
    const id = nonEmptyString(item.id) ?? nonEmptyString(item.model)
    if (!id) continue
    const reasoningEfforts = parseReasoningEfforts(item)
    models.push({
      providerId,
      id,
      displayName: stringOrNull(item.name) ?? id,
      description: stringOrNull(item.description) ?? undefined,
      contextWindow: positiveOrDefault(numberOrNull(item.context_window), 200_000),
      outputLimit: null,
      supportsTools: true,
      supportsAttachments: true,
      supportsReasoning: item.supports_reasoning_effort === true || reasoningEfforts.length > 0 ? true : null,
      supportedThinkingEfforts: reasoningEfforts.length > 0 ? reasoningEfforts : null,
      defaultThinkingEffort: defaultReasoningEffort(item, reasoningEfforts),
      canDisableThinking: null,
      serviceTiers: null,
      defaultServiceTierId: null,
      sourceFetchedAt,
      stale: false,
    })
  }

  if (models.length === 0) return grokBuildFallbackModels(providerId)

  return {
    providerId,
    models,
    defaultModelId: models[0]?.id ?? null,
    warnings: [],
    sourceFetchedAt,
    stale: false,
  }
}

function modelsUrl(baseUrl: string): string {
  const normalized = normalizeBaseUrl(baseUrl)
  return normalized.endsWith('/models') ? normalized : `${normalized}/models`
}

function parseReasoningEfforts(item: Record<string, unknown>): string[] {
  if (!Array.isArray(item.reasoning_efforts)) return []
  const ids: string[] = []
  for (const entry of item.reasoning_efforts) {
    if (!isRecord(entry)) continue
    const id = nonEmptyString(entry.id) ?? nonEmptyString(entry.value)
    if (id) ids.push(id)
  }
  return ids
}

function defaultReasoningEffort(item: Record<string, unknown>, efforts: string[]): string | null {
  if (Array.isArray(item.reasoning_efforts)) {
    for (const entry of item.reasoning_efforts) {
      if (isRecord(entry) && entry.default === true) {
        return nonEmptyString(entry.id) ?? nonEmptyString(entry.value) ?? null
      }
    }
  }
  const advertised = nonEmptyString(item.reasoning_effort)
  if (advertised) return advertised
  return efforts[0] ?? null
}

function positiveOrDefault(value: number | null, fallback: number): number {
  return value !== null && Number.isFinite(value) && value > 0 ? Math.trunc(value) : fallback
}
