import { expect, test } from 'bun:test'
import {
  listClaudeCodeModels,
  modelsDevAnthropicCatalogToModelList,
  parseClaudeModelVersion,
  resetClaudeCodeModelCatalogCacheForTests,
} from '../models'

test('parseClaudeModelVersion handles Claude full ids and date snapshots', () => {
  expect(parseClaudeModelVersion('claude-opus-4-8')).toEqual({ major: 4, minor: 8 })
  expect(parseClaudeModelVersion('claude-opus-4-5-20251101')).toEqual({ major: 4, minor: 5 })
  expect(parseClaudeModelVersion('claude-fable-5')).toEqual({ major: 5, minor: 0 })
  expect(parseClaudeModelVersion('claude-3-5-sonnet-20241022')).toEqual({ major: 3, minor: 5 })
  expect(parseClaudeModelVersion('claude-sonnet-4-20250514')).toEqual({ major: 4, minor: 0 })
  expect(parseClaudeModelVersion('not-claude')).toBeNull()
})

test('models.dev catalog keeps Claude full ids at or above minimum version without family allowlists', () => {
  const list = modelsDevAnthropicCatalogToModelList(modelsDevFixture(), {
    minimumModelVersion: '4.6',
    sourceFetchedAt: '2026-06-20T00:00:00.000Z',
  })

  expect(list.providerId).toBe('claude-code')
  expect(list.defaultModelId).toBeNull()
  expect(list.stale).toBe(false)
  expect(list.models.map((model) => model.id)).toEqual([
    'claude-opus-4-8',
    'claude-sonnet-4-6',
    'claude-fable-5',
    'claude-newfamily-5',
  ])
  expect(list.models.map((model) => model.id)).not.toContain('claude-haiku-4-5')
  expect(list.models.map((model) => model.id)).not.toContain('claude-3-5-sonnet-20241022')
  expect(list.models.every((model) => !model.id.startsWith('anthropic/'))).toBe(true)
})

test('models.dev mapping preserves explicit metadata and leaves missing capabilities null', () => {
  const list = modelsDevAnthropicCatalogToModelList(modelsDevFixture(), {
    minimumModelVersion: '4.6',
    sourceFetchedAt: '2026-06-20T00:00:00.000Z',
  })

  expect(list.models.find((model) => model.id === 'claude-opus-4-8')).toMatchObject({
    providerId: 'claude-code',
    id: 'claude-opus-4-8',
    displayName: 'Claude Opus 4.8',
    contextWindow: 1_000_000,
    outputLimit: 128_000,
    supportsTools: true,
    supportsAttachments: true,
    supportsReasoning: true,
    supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'],
    defaultThinkingEffort: null,
    cost: { input: 5, output: 25, cacheRead: 0.5, cacheWrite: 6.25 },
    sourceFetchedAt: '2026-06-20T00:00:00.000Z',
    stale: false,
  })
  expect(list.models.find((model) => model.id === 'claude-newfamily-5')).toMatchObject({
    contextWindow: null,
    outputLimit: null,
    supportsTools: null,
    supportsAttachments: null,
    supportsReasoning: null,
  })
})

test('listClaudeCodeModels uses stale cache on network failure and never returns hardcoded fallback models', async () => {
  resetClaudeCodeModelCatalogCacheForTests()
  const now = new Date('2026-06-20T00:00:00.000Z')
  const fetches: string[] = []
  const first = await listClaudeCodeModels({
    now: () => now,
    fetch: async (url) => {
      fetches.push(String(url))
      return Response.json(modelsDevFixture(), { headers: { etag: 'fixture-etag' } })
    },
  })
  expect(first.models.map((model) => model.id)).toContain('claude-opus-4-8')
  const freshCache = await listClaudeCodeModels({
    now: () => new Date(now.getTime() + 1_000),
    fetch: async () => {
      throw new Error('fresh cache should not fetch')
    },
  })
  expect(freshCache.stale).toBe(false)
  expect(freshCache.models.every((model) => !model.stale)).toBe(true)

  const stale = await listClaudeCodeModels({
    now: () => new Date(now.getTime() + 25 * 60 * 60 * 1000),
    fetch: async () => {
      throw new Error('offline')
    },
  })
  expect(fetches).toHaveLength(1)
  expect(stale.stale).toBe(true)
  expect(stale.models.every((model) => model.stale)).toBe(true)
  expect(stale.warnings.join('\n')).toContain('offline')
  expect(stale.models.map((model) => model.id)).toEqual(first.models.map((model) => model.id))
  resetClaudeCodeModelCatalogCacheForTests()
})

test('listClaudeCodeModels keeps cache entries isolated by minimum version', async () => {
  resetClaudeCodeModelCatalogCacheForTests()
  const now = new Date('2026-06-20T00:00:00.000Z')
  let calls = 0
  const fetchFixture = async () => {
    calls += 1
    return Response.json(modelsDevFixture(), { headers: { etag: `fixture-etag-${calls}` } })
  }

  const defaultList = await listClaudeCodeModels({
    minimumModelVersion: '4.6',
    now: () => now,
    fetch: fetchFixture,
  })
  const majorFiveList = await listClaudeCodeModels({
    minimumModelVersion: '5.0',
    now: () => now,
    fetch: fetchFixture,
  })

  expect(calls).toBe(2)
  expect(defaultList.models.map((model) => model.id)).toContain('claude-sonnet-4-6')
  expect(majorFiveList.models.map((model) => model.id)).toEqual(['claude-fable-5', 'claude-newfamily-5'])
  resetClaudeCodeModelCatalogCacheForTests()
})

function modelsDevFixture(): unknown {
  return {
    anthropic: {
      models: {
        // Intentionally unsorted (mimics models.dev's arbitrary key order) to exercise catalog ordering.
        'claude-fable-5': {
          name: 'Claude Fable 5',
          limit: { context: 1_000_000, output: 128_000 },
          reasoning: true,
          reasoning_options: [
            { type: 'budget_tokens', min: 1024 },
          ],
        },
        'claude-haiku-4-5': {
          name: 'Claude Haiku 4.5',
          limit: { context: 200_000, output: 64_000 },
          reasoning: true,
          attachment: true,
        },
        'claude-opus-4-8': {
          name: 'Claude Opus 4.8',
          limit: { context: 1_000_000, output: 128_000 },
          reasoning: true,
          reasoning_options: [
            { type: 'effort', values: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'] },
          ],
          attachment: true,
          tool_call: true,
          cost: { input: 5, output: 25, cache_read: 0.5, cache_write: 6.25 },
        },
        'claude-newfamily-5': {
          name: 'Claude Newfamily 5',
        },
        'claude-3-5-sonnet-20241022': {
          name: 'Claude Sonnet 3.5',
          limit: { context: 200_000, output: 8_192 },
        },
        'claude-sonnet-4-6': {
          name: 'Claude Sonnet 4.6',
          limit: { context: 1_000_000, output: 64_000 },
          reasoning: true,
          attachment: true,
          reasoning_options: [
            { type: 'effort', values: ['low', 'medium', 'high', 'max'] },
          ],
        },
        'not-claude-9': {
          name: 'Other provider model',
        },
      },
    },
  }
}
