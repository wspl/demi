import { normalizeBaseUrl } from '@demicodes/utils'
import { STATIC_CATALOG_SOURCE_DATE, parseProviderData, parseProviderJson, type ProviderModel, type ProviderModelList } from '@demicodes/provider'
import type { GrokAuthStore, GrokResolvedAuth } from './auth'
import { FileGrokAuthStore, GrokAuthError } from './auth'
import { DEFAULT_GROK_BUILD_BASE_URL, buildGrokBuildHeaders } from './headers'
import { grokModelsResponseSchema, type GrokModelsResponse } from './model-schemas'

export interface GrokBuildModelCatalogOptions {
  providerId?: string
  grokHome?: string
  baseUrl?: string
  clientVersion?: string
  authStore?: GrokAuthStore
  fetch?: (
    input: string | URL | Request,
    init?: RequestInit
  ) => Promise<Response>
}

export function grokBuildFallbackModels(
  providerId = 'grok-build'
): ProviderModelList {
  const sourceFetchedAt = STATIC_CATALOG_SOURCE_DATE
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

export async function listGrokBuildModels(
  options: GrokBuildModelCatalogOptions = {}
): Promise<ProviderModelList> {
  const providerId = options.providerId ?? 'grok-build'
  const authStore = options.authStore ?? new FileGrokAuthStore({
    grokHome: options.grokHome
  })
  const fetchImpl = options.fetch ?? fetch
  const baseUrl = normalizeBaseUrl(options.baseUrl
    ?? DEFAULT_GROK_BUILD_BASE_URL)

  let auth: GrokResolvedAuth
  try {
    auth = await authStore.resolveAuth()
  } catch (error) {
    if (error instanceof GrokAuthError && error.code === 'auth_missing') {
      return fallbackWithWarning(providerId, 'Grok credentials are missing')
    }
    throw error
  }
  const headers = buildGrokBuildHeaders(auth, undefined, {
    clientVersion: options.clientVersion,
    grokHome: options.grokHome,
  })
  let response: Response
  try {
    response = await fetchImpl(modelsUrl(baseUrl), { method: 'GET', headers })
  } catch {
    return fallbackWithWarning(providerId, 'Grok catalog network request failed')
  }
  if (!response.ok) {
    await response.body?.cancel().catch(() => {
      // An errored body is already closed.
    })
    if (response.status === 401 || response.status === 403) {
      throw new GrokAuthError('auth_invalid', `Grok model catalog authorization failed (HTTP ${response.status})`)
    }
    return fallbackWithWarning(providerId, `Grok model catalog returned HTTP ${response.status}`)
  }
  const payload = parseProviderJson(grokModelsResponseSchema, await response.text(), 'Grok model catalog')
  return mapGrokModels(payload, providerId)
}

function fallbackWithWarning(providerId: string, reason: string): ProviderModelList {
  return {
    ...grokBuildFallbackModels(providerId),
    warnings: [`${reason}; using fallback Grok catalog`],
  }
}

export function modelListFromGrokModelsPayload(
  payload: unknown,
  providerId: string
): ProviderModelList {
  return mapGrokModels(parseProviderData(grokModelsResponseSchema, payload, 'Grok model catalog'), providerId)
}

function mapGrokModels(payload: GrokModelsResponse, providerId: string): ProviderModelList {
  const sourceFetchedAt = new Date().toISOString()
  const models: ProviderModel[] = payload.data.map((item) => {
    const efforts = item.reasoning_efforts?.map((effort) => effort.id) ?? null
    return {
      providerId,
      id: item.id,
      displayName: item.name ?? item.id,
      description: item.description,
      contextWindow: item.context_window ?? null,
      outputLimit: null,
      supportsTools: null,
      supportsAttachments: item.input_modalities?.includes('image') ?? null,
      supportsReasoning: item.supports_reasoning_effort ?? (efforts?.length ? true : null),
      supportedThinkingEfforts: efforts,
      defaultThinkingEffort: item.reasoning_effort
        ?? item.reasoning_efforts?.find((effort) => effort.default)?.id ?? null,
      canDisableThinking: null,
      serviceTiers: null,
      defaultServiceTierId: null,
      sourceFetchedAt,
      stale: false,
    }
  })
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
