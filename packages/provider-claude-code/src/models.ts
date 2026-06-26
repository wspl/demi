import { errorMessage, isRecord, nonEmptyString, numberOrNull } from '@demi/utils'
import type { ProviderModel, ProviderModelList } from '@demi/provider'

export interface ClaudeCodeModelCatalogOptions {
  fetch?: ModelCatalogFetch
  modelsDevUrl?: string
  minimumModelVersion?: string
  now?: () => Date
}

export type ModelCatalogFetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>

interface ClaudeVersion {
  major: number
  minor: number
}

interface ClaudeCatalogCache {
  cacheKey: string
  etag: string | null
  lastModified: string | null
  fetchedAtMs: number
  list: ProviderModelList
}

const DEFAULT_MODELS_DEV_URL = 'https://models.dev/api.json'
const DEFAULT_MINIMUM_MODEL_VERSION = '4.6'
const MODELS_DEV_CACHE_TTL_MS = 24 * 60 * 60 * 1000

let memoryCache: ClaudeCatalogCache | null = null

export async function listClaudeCodeModels(options: ClaudeCodeModelCatalogOptions = {}): Promise<ProviderModelList> {
  const fetchImpl = options.fetch ?? fetch
  const now = options.now ?? (() => new Date())
  const nowDate = now()
  const minimum = parseMinimumModelVersion(options.minimumModelVersion ?? DEFAULT_MINIMUM_MODEL_VERSION)
  const modelsDevUrl = options.modelsDevUrl ?? DEFAULT_MODELS_DEV_URL
  const cacheKey = claudeCatalogCacheKey(modelsDevUrl, minimum)
  const headers = new Headers({ accept: 'application/json' })

  if (memoryCache?.cacheKey === cacheKey && nowDate.getTime() - memoryCache.fetchedAtMs < MODELS_DEV_CACHE_TTL_MS) {
    return cloneModelList(markModelListCache(memoryCache.list, false))
  }
  if (memoryCache?.cacheKey === cacheKey && memoryCache.etag) headers.set('if-none-match', memoryCache.etag)
  if (memoryCache?.cacheKey === cacheKey && memoryCache.lastModified) headers.set('if-modified-since', memoryCache.lastModified)

  try {
    const response = await fetchImpl(modelsDevUrl, { headers })
    if (response.status === 304 && memoryCache?.cacheKey === cacheKey) {
      memoryCache = { ...memoryCache, fetchedAtMs: nowDate.getTime() }
      return cloneModelList(markModelListCache(memoryCache.list, false))
    }
    if (!response.ok) throw new Error(`models.dev catalog request failed with HTTP ${response.status}`)

    const json = await response.json()
    const list = modelsDevAnthropicCatalogToModelList(json, {
      minimumModelVersion: minimum,
      sourceFetchedAt: nowDate.toISOString(),
      stale: false,
      warnings: [],
    })
    memoryCache = {
      cacheKey,
      etag: response.headers.get('etag'),
      lastModified: response.headers.get('last-modified'),
      fetchedAtMs: nowDate.getTime(),
      list,
    }
    return cloneModelList(list)
  } catch (error) {
    if (!memoryCache || memoryCache.cacheKey !== cacheKey) throw error
    const list = markModelListCache(memoryCache.list, true)
    return {
      ...cloneModelList(list),
      warnings: [...list.warnings, `Using stale models.dev catalog: ${errorMessage(error)}`],
    }
  }
}

export function modelsDevAnthropicCatalogToModelList(
  value: unknown,
  options: {
    minimumModelVersion?: string | ClaudeVersion
    sourceFetchedAt?: string
    stale?: boolean
    warnings?: string[]
  } = {},
): ProviderModelList {
  const sourceFetchedAt = options.sourceFetchedAt ?? new Date().toISOString()
  const minimum =
    typeof options.minimumModelVersion === 'string'
      ? parseMinimumModelVersion(options.minimumModelVersion)
      : options.minimumModelVersion ?? parseMinimumModelVersion(DEFAULT_MINIMUM_MODEL_VERSION)
  const warnings = [...(options.warnings ?? [])]
  const anthropic = isRecord(value) && isRecord(value.anthropic) ? value.anthropic : null
  const rawModels = anthropic && isRecord(anthropic.models) ? anthropic.models : null
  if (!rawModels) throw new Error('models.dev response does not contain anthropic.models')

  const models: ProviderModel[] = []
  for (const [id, rawModel] of Object.entries(rawModels)) {
    if (!id.startsWith('claude-')) continue
    const version = parseClaudeModelVersion(id)
    if (!version) {
      warnings.push(`Skipped Claude model with unparseable version: ${id}`)
      continue
    }
    if (!versionGte(version, minimum)) continue
    if (!isRecord(rawModel)) {
      warnings.push(`Skipped Claude model with invalid metadata: ${id}`)
      continue
    }
    models.push(modelFromModelsDevEntry(id, rawModel, sourceFetchedAt, options.stale === true))
  }

  models.sort(compareClaudeModels)

  return {
    providerId: 'claude-code',
    models,
    defaultModelId: null,
    warnings,
    sourceFetchedAt,
    stale: options.stale === true,
  }
}

const CLAUDE_FAMILY_RANK: Record<string, number> = { opus: 0, sonnet: 1, haiku: 2 }

function claudeFamilyRank(id: string): number {
  const family = id.slice('claude-'.length).split('-')[0] ?? ''
  return CLAUDE_FAMILY_RANK[family] ?? 3
}

/** Canonical catalog order: flagship family first (Opus > Sonnet > Haiku > others), newest version first. */
function compareClaudeModels(a: ProviderModel, b: ProviderModel): number {
  const familyDelta = claudeFamilyRank(a.id) - claudeFamilyRank(b.id)
  if (familyDelta !== 0) return familyDelta
  const versionA = parseClaudeModelVersion(a.id)
  const versionB = parseClaudeModelVersion(b.id)
  if (versionA && versionB) {
    if (versionA.major !== versionB.major) return versionB.major - versionA.major
    if (versionA.minor !== versionB.minor) return versionB.minor - versionA.minor
  }
  return a.id.localeCompare(b.id)
}

export function parseClaudeModelVersion(id: string): ClaudeVersion | null {
  if (!id.startsWith('claude-')) return null
  const parts = id.slice('claude-'.length).split('-')
  const isInteger = (value: string | undefined): value is string => value !== undefined && /^\d+$/.test(value)
  const isDate = (value: string | undefined): boolean => value !== undefined && /^\d{8}$/.test(value)

  if (isInteger(parts[0])) {
    return {
      major: Number(parts[0]),
      minor: isInteger(parts[1]) ? Number(parts[1]) : 0,
    }
  }
  if (!isInteger(parts[1])) return null
  return {
    major: Number(parts[1]),
    minor: isInteger(parts[2]) && !isDate(parts[2]) ? Number(parts[2]) : 0,
  }
}

export function resetClaudeCodeModelCatalogCacheForTests(): void {
  memoryCache = null
}

function modelFromModelsDevEntry(id: string, raw: Record<string, unknown>, sourceFetchedAt: string, stale: boolean): ProviderModel {
  const limit = isRecord(raw.limit) ? raw.limit : null
  const cost = isRecord(raw.cost) ? raw.cost : null
  return {
    providerId: 'claude-code',
    id,
    displayName: nonEmptyString(raw.name) ?? id,
    description: nonEmptyString(raw.description),
    contextWindow: numberOrNull(limit?.context),
    outputLimit: numberOrNull(limit?.output),
    supportsTools: booleanOrNull(isRecord(raw.tool) ? raw.tool.call : raw.tool_call),
    supportsAttachments: booleanOrNull(raw.attachment),
    supportsReasoning: booleanOrNull(raw.reasoning),
    supportedThinkingEfforts: reasoningEfforts(raw.reasoning_options),
    defaultThinkingEffort: null,
    // The `claude` CLI's --effort flag only levels thinking (low|medium|high|xhigh|max); it can't
    // turn it off, so thinking is never fully disableable for Claude Code models.
    canDisableThinking: false,
    ...(cost
      ? {
          cost: {
            input: numberOrNull(cost.input),
            output: numberOrNull(cost.output),
            cacheRead: numberOrNull(cost.cache_read),
            cacheWrite: numberOrNull(cost.cache_write),
          },
        }
      : {}),
    sourceFetchedAt,
    stale,
  }
}

function parseMinimumModelVersion(value: string): ClaudeVersion {
  const match = /^(\d+)(?:\.(\d+))?$/.exec(value)
  if (!match) throw new Error(`Invalid minimum Claude model version: ${value}`)
  return { major: Number(match[1]), minor: match[2] ? Number(match[2]) : 0 }
}

function versionGte(version: ClaudeVersion, minimum: ClaudeVersion): boolean {
  return version.major > minimum.major || (version.major === minimum.major && version.minor >= minimum.minor)
}

function claudeCatalogCacheKey(modelsDevUrl: string, minimum: ClaudeVersion): string {
  return `${modelsDevUrl}\0${minimum.major}.${minimum.minor}`
}

function markModelListCache(list: ProviderModelList, stale: boolean): ProviderModelList {
  return {
    ...list,
    sourceFetchedAt: list.sourceFetchedAt,
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
    })),
  }
}


function booleanOrNull(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null
}

function reasoningEfforts(value: unknown): ProviderModel['supportedThinkingEfforts'] {
  if (!Array.isArray(value)) return null
  const effortOption = value.find((option) => isRecord(option) && option.type === 'effort')
  if (!isRecord(effortOption) || !Array.isArray(effortOption.values)) return null
  const efforts = effortOption.values.map((effort) => nonEmptyString(effort)).filter((effort): effort is string => effort !== undefined)
  return efforts.length > 0 ? efforts : []
}
