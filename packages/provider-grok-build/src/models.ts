import { normalizeBaseUrl } from '@demicodes/utils'
import { z } from 'zod'
import type { ProviderModel, ProviderModelList } from '@demicodes/provider'
import type { GrokAuthStore } from './auth'
import { FileGrokAuthStore } from './vendor'
import { DEFAULT_GROK_BUILD_BASE_URL, buildGrokBuildHeaders } from './headers'

export interface GrokBuildModelCatalogOptions {
  signal?: AbortSignal
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

const FALLBACK_SOURCE_FETCHED_AT = '1970-01-01T00:00:00.000Z'
const FALLBACK_CONTEXT_WINDOW = 200_000

/**
 * One model of the Grok `/v1/models` catalog. Only the fields Demi maps are
 * described; a context window is a count of tokens, so it is a positive
 * integer. A reasoning effort is named by `id` or, on older deployments, by
 * `value`.
 */
const grokCatalogModelSchema = z.looseObject({
  id: z.string().min(1).optional(),
  model: z.string().min(1).optional(),
  name: z.string().min(1).optional(),
  description: z.string().optional(),
  context_window: z.number().int().positive().optional(),
  supports_reasoning_effort: z.boolean().optional(),
  reasoning_effort: z.string().min(1).optional(),
  reasoning_efforts: z.array(z.looseObject({
    id: z.string().min(1).optional(),
    value: z.string().min(1).optional(),
    default: z.boolean().optional(),
  })).optional(),
})

type GrokCatalogModel = z.infer<typeof grokCatalogModelSchema>

/** The catalog, as an OpenAI-style envelope or as the bare list. */
const grokModelsPayloadSchema = z.union([
  z.looseObject({ data: z.array(grokCatalogModelSchema).optional() }),
  z.array(grokCatalogModelSchema),
])

export function grokBuildFallbackModels(
  providerId = 'grok-build'
): ProviderModelList {
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

  let response: Response
  try {
    const auth = await authStore.resolveAuth()
    response = await fetchImpl(modelsUrl(baseUrl), {
      method: 'GET',
      signal: options.signal,
      headers: buildGrokBuildHeaders(auth, undefined, {
        clientVersion: options.clientVersion,
        grokHome: options.grokHome,
      }),
    })
  } catch {
    // No session or no route to the vendor: the fallback catalog keeps the
    // model picker usable offline.
    options.signal?.throwIfAborted()
    return grokBuildFallbackModels(providerId)
  }
  if (!response.ok) {
    const fallback = grokBuildFallbackModels(providerId)
    return {
      ...fallback,
      warnings: [`Grok Build /v1/models returned HTTP ${response.status}; using fallback catalog`],
    }
  }
  // A vendor that answers with a catalog Demi cannot read is a protocol error,
  // not an unreachable vendor: report it instead of silently substituting one.
  return modelListFromGrokModelsPayload(await response.json(), providerId)
}

export function modelListFromGrokModelsPayload(
  payload: unknown,
  providerId: string
): ProviderModelList {
  const sourceFetchedAt = new Date().toISOString()
  const decoded = grokModelsPayloadSchema.parse(payload)
  const data = Array.isArray(decoded) ? decoded : decoded.data ?? []
  const models: ProviderModel[] = []

  for (const item of data) {
    const id = item.id ?? item.model
    if (!id)
      continue
    const reasoning = reasoningEffortsOf(item)
    models.push({
      providerId,
      id,
      displayName: item.name ?? id,
      description: item.description,
      contextWindow: item.context_window ?? FALLBACK_CONTEXT_WINDOW,
      outputLimit: null,
      supportsTools: true,
      // cli-chat-proxy omits modalities; Grok Build stock harness keeps native images.
      supportsAttachments: true,
      supportsReasoning: item.supports_reasoning_effort === true
        || reasoning.ids.length > 0
        ? true
        : null,
      supportedThinkingEfforts: reasoning.ids.length > 0
        ? reasoning.ids
        : null,
      defaultThinkingEffort: reasoning.defaultId,
      canDisableThinking: null,
      serviceTiers: null,
      defaultServiceTierId: null,
      sourceFetchedAt,
      stale: false,
    })
  }

  if (models.length === 0)
    return grokBuildFallbackModels(providerId)

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

/**
 * The reasoning efforts a model advertises and the one it starts on. An entry
 * flagged `default` decides; failing that, the model's own `reasoning_effort`,
 * failing that the first effort it lists.
 */
function reasoningEffortsOf(
  item: GrokCatalogModel
): { ids: string[]; defaultId: string | null } {
  const ids: string[] = []
  let flagged: string | undefined
  for (const entry of item.reasoning_efforts ?? []) {
    const id = entry.id ?? entry.value
    if (!id)
      continue
    ids.push(id)
    if (entry.default === true)
      flagged ??= id
  }
  return {
    ids,
    defaultId: flagged ?? item.reasoning_effort ?? ids[0] ?? null,
  }
}
