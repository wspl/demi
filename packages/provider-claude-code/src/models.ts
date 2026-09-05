import {
  fetchModelsDev,
  modelFromModelsDev,
  modelsDevCatalogSchema,
  resetModelsDevCacheForTests,
  type ModelsDevCatalog,
  type ModelsDevFetch,
  type ProviderModel,
  type ProviderModelList,
} from '@demicodes/provider'

export interface ClaudeCodeModelCatalogOptions {
  fetch?: ModelCatalogFetch
  modelsDevUrl?: string
  minimumModelVersion?: string
  now?: () => Date
}

export type ModelCatalogFetch = ModelsDevFetch

interface ClaudeVersion {
  major: number
  minor: number
}

const DEFAULT_MINIMUM_MODEL_VERSION = '4.6'

/**
 * The Claude Code catalog: models.dev's `anthropic` entries at or above the
 * minimum version (the CLI runs nothing older), flagship family first. The
 * fetch, its cache and the stale fallback are the shared models.dev client's.
 */
export async function listClaudeCodeModels(options: ClaudeCodeModelCatalogOptions = {}): Promise<ProviderModelList> {
  const snapshot = await fetchModelsDev({ fetch: options.fetch, url: options.modelsDevUrl, now: options.now })
  return modelsDevAnthropicCatalogToModelList(snapshot.catalog, {
    minimumModelVersion: options.minimumModelVersion ?? DEFAULT_MINIMUM_MODEL_VERSION,
    sourceFetchedAt: snapshot.fetchedAt,
    stale: snapshot.stale,
    warnings: snapshot.warnings,
  })
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
  const stale = options.stale === true
  const minimum =
    typeof options.minimumModelVersion === 'string'
      ? parseMinimumModelVersion(options.minimumModelVersion)
      : options.minimumModelVersion ?? parseMinimumModelVersion(DEFAULT_MINIMUM_MODEL_VERSION)
  const warnings = [...(options.warnings ?? [])]
  const catalog: ModelsDevCatalog = modelsDevCatalogSchema.parse(value)
  const anthropic = catalog.anthropic
  if (!anthropic) throw new Error('models.dev response does not contain anthropic.models')

  const models: ProviderModel[] = []
  for (const [id, entry] of Object.entries(anthropic.models)) {
    if (!id.startsWith('claude-')) continue
    const version = parseClaudeModelVersion(id)
    if (!version) {
      warnings.push(`Skipped Claude model with unparseable version: ${id}`)
      continue
    }
    if (!versionGte(version, minimum)) continue
    models.push({
      ...modelFromModelsDev('claude-code', id, entry, { sourceFetchedAt, stale }),
      // The `claude` CLI's --effort flag only levels thinking (low|medium|high|xhigh|max); it can't
      // turn it off, so thinking is never fully disableable for Claude Code models.
      canDisableThinking: false,
    })
  }

  models.sort(compareClaudeModels)

  return {
    providerId: 'claude-code',
    models,
    defaultModelId: null,
    warnings,
    sourceFetchedAt,
    stale,
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
  resetModelsDevCacheForTests()
}

function parseMinimumModelVersion(value: string): ClaudeVersion {
  const match = /^(\d+)(?:\.(\d+))?$/.exec(value)
  if (!match) throw new Error(`Invalid minimum Claude model version: ${value}`)
  return { major: Number(match[1]), minor: match[2] ? Number(match[2]) : 0 }
}

function versionGte(version: ClaudeVersion, minimum: ClaudeVersion): boolean {
  return version.major > minimum.major || (version.major === minimum.major && version.minor >= minimum.minor)
}
