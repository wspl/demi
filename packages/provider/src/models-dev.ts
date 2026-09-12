// The models.dev catalog client: one fetch of `https://models.dev/api.json`
// (etag / last-modified revalidation, a day of freshness, the last good copy
// served stale when the network fails) and the mapping from its entries onto
// the provider kit's model catalog shape. Consumers filter the catalog for
// what they need — a vendor's model list, one vendor's entries above a
// version — the client knows nothing about vendors.
import { errorMessage } from '@demicodes/utils'
import { parseProviderJson } from './validation'
import { z } from 'zod'
import type {
  ProviderModel,
  ProviderModelList,
  ProviderModelListOptions
} from './types'

const modelsDevReasoningOptionSchema = z.looseObject({
  type: z.string().min(1),
  values: z.array(z.string().min(1).nullable()).optional(),
})

export const modelsDevModelSchema = z.looseObject({
  name: z.string().min(1).optional(),
  description: z.string().optional(),
  attachment: z.boolean().optional(),
  reasoning: z.boolean().optional(),
  reasoning_options: z.array(modelsDevReasoningOptionSchema).optional(),
  tool_call: z.boolean().optional(),
  limit: z.looseObject({
    context: z.number().int().nonnegative().optional(),
    output: z.number().int().nonnegative().optional()
  }).optional(),
  cost: z
    .looseObject({
      input: z.number().nonnegative().optional(),
      output: z.number().nonnegative().optional(),
      cache_read: z.number().nonnegative().optional(),
      cache_write: z.number().nonnegative().optional(),
    })
    .optional(),
})

export const modelsDevProviderSchema = z.looseObject({
  id: z.string().min(1),
  name: z.string().min(1),
  /**
   * The client package the data is written for — the catalog's only protocol
   * tag.
   */
  npm: z.string().optional(),
  /**
   * The vendor's base URL; absent for the first-party vendors whose clients
   * know it.
   */
  api: z.string().optional(),
  doc: z.string().optional(),
  models: z.record(z.string().min(1), modelsDevModelSchema),
})

export const modelsDevCatalogSchema = z.record(
  z.string().min(1),
  modelsDevProviderSchema
)

export type ModelsDevModel = z.infer<typeof modelsDevModelSchema>
export type ModelsDevProvider = z.infer<typeof modelsDevProviderSchema>
export type ModelsDevCatalog = z.infer<typeof modelsDevCatalogSchema>

export type ModelsDevFetch = (
  input: string | URL | Request,
  init?: RequestInit
) => Promise<Response>

export interface ModelsDevOptions extends ProviderModelListOptions {
  fetch?: ModelsDevFetch
  url?: string
  now?: () => Date
}

/**
 * A catalog snapshot: the data, when it was fetched, and whether it is the
 * stale copy after a failed refresh.
 */
export interface ModelsDevSnapshot {
  catalog: ModelsDevCatalog
  fetchedAt: string
  stale: boolean
  warnings: string[]
}

interface ModelsDevCache {
  url: string
  etag: string | null
  lastModified: string | null
  fetchedAtMs: number
  fetchedAt: string
  catalog: ModelsDevCatalog
}

export const DEFAULT_MODELS_DEV_URL = 'https://models.dev/api.json'
const MODELS_DEV_CACHE_TTL_MS = 24 * 60 * 60 * 1000

let cache: ModelsDevCache | null = null

/**
 * The current models.dev catalog: the fresh in-memory copy, a revalidated
 * one, or the last good copy marked stale when the refresh fails. Throws
 * only when there is no copy at all.
 */
export async function fetchModelsDev(
  options: ModelsDevOptions = {}
): Promise<ModelsDevSnapshot> {
  const fetchImpl = options.fetch ?? fetch
  const url = options.url ?? DEFAULT_MODELS_DEV_URL
  const nowDate = (options.now ?? (() => new Date()))()
  const cached = cache?.url === url ? cache : null

  if (!options.refresh && cached
    && nowDate.getTime() - cached.fetchedAtMs < MODELS_DEV_CACHE_TTL_MS) {
    return {
      catalog: structuredClone(cached.catalog),
      fetchedAt: cached.fetchedAt,
      stale: false,
      warnings: []
    }
  }
  const headers = new Headers({ accept: 'application/json' })
  if (cached?.etag)
    headers.set('if-none-match', cached.etag)
  if (cached?.lastModified)
    headers.set(
      'if-modified-since',
      cached.lastModified
    )

  try {
    const response = await fetchImpl(url, { headers })
    if (response.status === 304 && cached) {
      cache = { ...cached, fetchedAtMs: nowDate.getTime() }
      return {
        catalog: structuredClone(cached.catalog),
        fetchedAt: cached.fetchedAt,
        stale: false,
        warnings: []
      }
    }
    if (!response.ok) {
      await response.body?.cancel().catch(() => {
        // An errored response body is already closed.
      })
      throw new Error(`models.dev catalog request failed with HTTP ${response.status}`)
    }
    const catalog = parseProviderJson(modelsDevCatalogSchema, await response.text(), 'models.dev catalog')
    const fetchedAt = nowDate.toISOString()
    cache = {
      url,
      etag: response.headers.get('etag'),
      lastModified: response.headers.get('last-modified'),
      fetchedAtMs: nowDate.getTime(),
      fetchedAt,
      catalog,
    }
    return { catalog: structuredClone(catalog), fetchedAt, stale: false, warnings: [] }
  } catch (error) {
    if (!cached)
      throw error
    return {
      catalog: structuredClone(cached.catalog),
      fetchedAt: cached.fetchedAt,
      stale: true,
      warnings: [`Using stale models.dev catalog: ${errorMessage(error)}`],
    }
  }
}

export function resetModelsDevCacheForTests(): void {
  cache = null
}

/**
 * One models.dev model entry as a catalog model under `providerId`;
 * capabilities the entry omits are null (unknown).
 */
export function modelFromModelsDev(
  providerId: string,
  id: string,
  entry: ModelsDevModel,
  meta: {
    sourceFetchedAt: string;
    stale: boolean
  },
): ProviderModel {
  return {
    providerId,
    id,
    displayName: entry.name ?? id,
    description: entry.description,
    contextWindow: entry.limit?.context ?? null,
    outputLimit: entry.limit?.output ?? null,
    supportsTools: entry.tool_call ?? null,
    supportsAttachments: entry.attachment ?? null,
    supportsReasoning: entry.reasoning ?? null,
    supportedThinkingEfforts: reasoningEfforts(entry.reasoning_options),
    defaultThinkingEffort: null,
    ...(entry.cost
      ? {
          cost: {
            input: entry.cost.input ?? null,
            output: entry.cost.output ?? null,
            cacheRead: entry.cost.cache_read ?? null,
            cacheWrite: entry.cost.cache_write ?? null,
          },
        }
      : {}),
    sourceFetchedAt: meta.sourceFetchedAt,
    stale: meta.stale,
  }
}

/**
 * A vendor's whole model list from a snapshot, as a catalog under `providerId`;
 * null when the vendor is unknown.
 */
export function modelListFromModelsDev(
  snapshot: ModelsDevSnapshot,
  vendorId: string,
  providerId: string
): ProviderModelList | null {
  const vendor = snapshot.catalog[vendorId]
  if (!vendor)
    return null
  const meta = { sourceFetchedAt: snapshot.fetchedAt, stale: snapshot.stale }
  return {
    providerId,
    models: Object.entries(vendor.models).map(
      ([id, entry]) => modelFromModelsDev(
        providerId,
        id,
        entry,
        meta
      )
    ),
    defaultModelId: null,
    warnings: [...snapshot.warnings],
    sourceFetchedAt: snapshot.fetchedAt,
    stale: snapshot.stale,
  }
}

function reasoningEfforts(
  options: ModelsDevModel['reasoning_options']
): ProviderModel['supportedThinkingEfforts'] {
  const effort = options?.find((option) => option.type === 'effort')
  if (!effort?.values)
    return null
  const efforts = effort.values.filter(
    (value): value is string => value !== null
  )
  return efforts
}
