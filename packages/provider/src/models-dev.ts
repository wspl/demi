// The models.dev catalog client: one fetch of `https://models.dev/api.json`
// (etag / last-modified revalidation, a day of freshness, the last good copy
// served stale when the network fails) and the mapping from its entries onto
// the provider kit's model catalog shape. Consumers filter the catalog for
// what they need — a vendor's model list, one vendor's entries above a
// version — the client knows nothing about vendors.
import { errorMessage, numberOrNull } from '@demicodes/utils'
import { z } from 'zod'
import type { ProviderModel, ProviderModelList } from './types'

const modelsDevReasoningOptionSchema = z.looseObject({
  type: z.string(),
  values: z.array(z.string().nullable()).optional(),
})

export const modelsDevModelSchema = z.looseObject({
  name: z.string().optional(),
  description: z.string().optional(),
  attachment: z.boolean().optional(),
  reasoning: z.boolean().optional(),
  reasoning_options: z.array(modelsDevReasoningOptionSchema).optional(),
  tool_call: z.boolean().optional(),
  limit: z.looseObject({ context: z.number().optional(), output: z.number().optional() }).optional(),
  cost: z
    .looseObject({
      input: z.number().optional(),
      output: z.number().optional(),
      cache_read: z.number().optional(),
      cache_write: z.number().optional(),
    })
    .optional(),
})

export const modelsDevProviderSchema = z.looseObject({
  id: z.string(),
  name: z.string(),
  /** The client package the data is written for — the catalog's only protocol tag. */
  npm: z.string().optional(),
  /** The vendor's base URL; absent for the first-party vendors whose clients know it. */
  api: z.string().optional(),
  doc: z.string().optional(),
  models: z.record(z.string(), modelsDevModelSchema),
})

export const modelsDevCatalogSchema = z.record(z.string(), modelsDevProviderSchema)

export type ModelsDevModel = z.infer<typeof modelsDevModelSchema>
export type ModelsDevProvider = z.infer<typeof modelsDevProviderSchema>
export type ModelsDevCatalog = z.infer<typeof modelsDevCatalogSchema>

export type ModelsDevFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

export interface ModelsDevOptions {
  fetch?: ModelsDevFetch
  url?: string
  now?: () => Date
}

/** A catalog snapshot: the data, when it was fetched, and whether it is the stale copy after a failed refresh. */
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
export async function fetchModelsDev(options: ModelsDevOptions = {}): Promise<ModelsDevSnapshot> {
  const fetchImpl = options.fetch ?? fetch
  const url = options.url ?? DEFAULT_MODELS_DEV_URL
  const nowDate = (options.now ?? (() => new Date()))()
  const cached = cache?.url === url ? cache : null

  if (cached && nowDate.getTime() - cached.fetchedAtMs < MODELS_DEV_CACHE_TTL_MS) {
    return { catalog: cached.catalog, fetchedAt: cached.fetchedAt, stale: false, warnings: [] }
  }
  const headers = new Headers({ accept: 'application/json' })
  if (cached?.etag) headers.set('if-none-match', cached.etag)
  if (cached?.lastModified) headers.set('if-modified-since', cached.lastModified)

  try {
    const response = await fetchImpl(url, { headers })
    if (response.status === 304 && cached) {
      cache = { ...cached, fetchedAtMs: nowDate.getTime() }
      return { catalog: cached.catalog, fetchedAt: cached.fetchedAt, stale: false, warnings: [] }
    }
    if (!response.ok) throw new Error(`models.dev catalog request failed with HTTP ${response.status}`)
    const catalog = modelsDevCatalogSchema.parse(await response.json())
    const fetchedAt = nowDate.toISOString()
    cache = {
      url,
      etag: response.headers.get('etag'),
      lastModified: response.headers.get('last-modified'),
      fetchedAtMs: nowDate.getTime(),
      fetchedAt,
      catalog,
    }
    return { catalog, fetchedAt, stale: false, warnings: [] }
  } catch (error) {
    if (!cached) throw error
    return {
      catalog: cached.catalog,
      fetchedAt: cached.fetchedAt,
      stale: true,
      warnings: [`Using stale models.dev catalog: ${errorMessage(error)}`],
    }
  }
}

export function resetModelsDevCacheForTests(): void {
  cache = null
}

/** One models.dev model entry as a catalog model under `providerId`; capabilities the entry omits are null (unknown). */
export function modelFromModelsDev(
  providerId: string,
  id: string,
  entry: ModelsDevModel,
  meta: { sourceFetchedAt: string; stale: boolean },
): ProviderModel {
  return {
    providerId,
    id,
    displayName: entry.name ?? id,
    description: entry.description,
    contextWindow: numberOrNull(entry.limit?.context),
    outputLimit: numberOrNull(entry.limit?.output),
    supportsTools: entry.tool_call ?? null,
    supportsAttachments: entry.attachment ?? null,
    supportsReasoning: entry.reasoning ?? null,
    supportedThinkingEfforts: reasoningEfforts(entry.reasoning_options),
    defaultThinkingEffort: null,
    ...(entry.cost
      ? {
          cost: {
            input: numberOrNull(entry.cost.input),
            output: numberOrNull(entry.cost.output),
            cacheRead: numberOrNull(entry.cost.cache_read),
            cacheWrite: numberOrNull(entry.cost.cache_write),
          },
        }
      : {}),
    sourceFetchedAt: meta.sourceFetchedAt,
    stale: meta.stale,
  }
}

/** A vendor's whole model list from a snapshot, as a catalog under `providerId`; null when the vendor is unknown. */
export function modelListFromModelsDev(snapshot: ModelsDevSnapshot, vendorId: string, providerId: string): ProviderModelList | null {
  const vendor = snapshot.catalog[vendorId]
  if (!vendor) return null
  const meta = { sourceFetchedAt: snapshot.fetchedAt, stale: snapshot.stale }
  return {
    providerId,
    models: Object.entries(vendor.models).map(([id, entry]) => modelFromModelsDev(providerId, id, entry, meta)),
    defaultModelId: null,
    warnings: [...snapshot.warnings],
    sourceFetchedAt: snapshot.fetchedAt,
    stale: snapshot.stale,
  }
}

function reasoningEfforts(options: ModelsDevModel['reasoning_options']): ProviderModel['supportedThinkingEfforts'] {
  const effort = options?.find((option) => option.type === 'effort')
  if (!effort?.values) return null
  const efforts = effort.values.filter((value): value is string => typeof value === 'string' && value.length > 0)
  return efforts.length > 0 ? efforts : []
}
