import { afterEach, expect, test } from 'bun:test'
import { fetchModelsDev, resetModelsDevCacheForTests } from '../models-dev'

afterEach(resetModelsDevCacheForTests)
const catalog = {
  vendor: { id: 'vendor', name: 'Vendor', models: { model: { name: 'Model' } } }
}

test(
  'explicit refresh revalidates a fresh catalog and preserves content date on 304',
  async () => {
    const requests: Headers[] = []
    let now = new Date('2026-09-08T00:00:00Z')
    const options = {
      now: () => now,
      fetch: async (_input: string | URL | Request, init?: RequestInit) => {
        requests.push(new Headers(init?.headers))
        return requests.length === 1
          ? Response.json(catalog, { headers: { etag: 'v1' } })
          : new Response(null, { status: 304 })
      },
    }
    const first = await fetchModelsDev(options)
    now = new Date('2026-09-08T00:01:00Z')
    expect(await fetchModelsDev(options)).toEqual(first)
    expect(requests).toHaveLength(1)
    const refreshed = await fetchModelsDev({ ...options, refresh: true })
    expect(requests).toHaveLength(2)
    expect(requests[1]!.get('if-none-match')).toBe('v1')
    expect(refreshed).toEqual(first)
  }
)

test(
  'a forced refresh failure returns the old catalog explicitly marked stale',
  async () => {
    const first = await fetchModelsDev({
      fetch: async () => Response.json(catalog)
    })
    const stale = await fetchModelsDev({
      refresh: true,
      fetch: async () => {
        throw new Error('offline')
      }
    })
    expect(stale.catalog).toEqual(first.catalog)
    expect(stale.fetchedAt).toBe(first.fetchedAt)
    expect(stale.stale).toBe(true)
    expect(stale.warnings.join(' ')).toContain('offline')
  }
)

test(
  'refresh without an old catalog rejects malformed external data and network failures',
  async () => {
    await expect(fetchModelsDev({
      refresh: true,
      fetch: async () => Response.json({ vendor: { models: [] } })
    })).rejects.toThrow()
    await expect(fetchModelsDev({
      refresh: true,
      fetch: async () => {
        throw new Error('offline')
      }
    })).rejects.toThrow('offline')
  }
)

test('models.dev rejects malformed limits and costs, with no implicit number conversion', async () => {
  const { modelsDevCatalogSchema, modelFromModelsDev } = await import('../models-dev')
  for (const model of [
    { limit: { context: '100' } }, { limit: { context: -1 } }, { limit: { output: 1.5 } },
    { cost: { input: -1 } }, { cost: { output: NaN } }, { name: [] },
    { reasoning: 'true' }, { reasoning_options: [{ type: 'effort', values: [''] }] },
  ]) {
    expect(modelsDevCatalogSchema.safeParse({ vendor: { id: 'vendor', name: 'Vendor', models: { m: model } } }).success)
      .toBe(false)
  }
  const parsed = modelsDevCatalogSchema.parse({ vendor: { id: 'vendor', name: 'Vendor', models: {
    m: { cost: { input: 0, output: 1.5 } },
  } } })
  expect(modelFromModelsDev('v', 'm', parsed.vendor!.models.m!, {
    sourceFetchedAt: '2026-09-12T00:00:00Z', stale: false,
  })).toMatchObject({ contextWindow: null, outputLimit: null, supportsAttachments: null,
    cost: { input: 0, output: 1.5, cacheRead: null, cacheWrite: null } })
})

test('models.dev snapshots cannot mutate cached validated data', async () => {
  const options = { fetch: async () => Response.json(catalog) }
  const first = await fetchModelsDev(options)
  first.catalog.vendor!.models.model!.name = 'changed by caller'
  expect((await fetchModelsDev(options)).catalog.vendor!.models.model!.name).toBe('Model')
  const stale = await fetchModelsDev({ refresh: true, fetch: async () => Response.json({
    vendor: { id: 'vendor', name: 'Vendor', models: { model: { limit: { context: 'private-value' } } } },
  }) })
  expect(stale.stale).toBe(true)
  expect(stale.catalog.vendor!.models.model!.name).toBe('Model')
  expect(stale.warnings[0]).toContain('limit.context')
  expect(stale.warnings[0]).not.toContain('private-value')
})
