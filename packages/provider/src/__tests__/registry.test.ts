import { expect, test } from 'bun:test'
import { ProviderRegistry, StubProvider, events, type ProviderDefinition } from '../index'

test('ProviderRegistry registers, looks up, and creates providers by type', async () => {
  const registry = new ProviderRegistry()
  const definition: ProviderDefinition<{ answer: string }> = {
    type: 'stub',
    displayName: 'Stub',
    state: () => ({ status: 'ready' }),
    createProvider: (config) => new StubProvider([[events.text(config.answer), events.response()]]),
  }

  registry.register(definition)

  expect(registry.get('stub')).toBe(definition)
  expect(await registry.state('stub')).toEqual({ status: 'ready' })

  const provider = await registry.createProvider('stub', { answer: 'ok' })
  const output = []
  for await (const event of provider.run({
    sessionId: 'test-session',
    turnId: 'test-turn',
    requestId: 'test-request',
    modelId: 'test-model',
    systemPrompt: '',
    cwd: '/tmp',
    items: [],
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
  })) {
    output.push(event)
  }

  expect(output[0]).toEqual(events.text('ok'))
})

test('ProviderRegistry emits snapshots when registrations change', () => {
  const registry = new ProviderRegistry()
  const seen: string[][] = []
  registry.observe((snapshot) => {
    seen.push(snapshot.providers.map((p) => p.type))
  })

  const unregister = registry.register({
    type: 'stub',
    displayName: 'Stub',
    createProvider: () => new StubProvider([[events.response()]]),
  })
  unregister()

  expect(seen).toEqual([[], ['stub'], []])
})

test('ProviderRegistry dispatches provider model catalogs', async () => {
  const registry = new ProviderRegistry()
  registry.register({
    type: 'stub',
    displayName: 'Stub',
    listModels: (config: { region: string }) => ({
      providerId: 'stub',
      defaultModelId: 'model-1',
      sourceFetchedAt: '2026-06-20T00:00:00.000Z',
      stale: false,
      warnings: [config.region],
      models: [
        {
          providerId: 'stub',
          id: 'model-1',
          displayName: 'Model 1',
          contextWindow: 100,
          outputLimit: null,
          supportsTools: null,
          supportsAttachments: null,
          supportsReasoning: null,
          supportedThinkingEfforts: null,
          defaultThinkingEffort: null,
          source: 'cache',
          sourceFetchedAt: '2026-06-20T00:00:00.000Z',
          stale: false,
        },
      ],
    }),
    createProvider: () => new StubProvider([[events.response()]]),
  })
  registry.register({
    type: 'no-catalog',
    displayName: 'No Catalog',
    createProvider: () => new StubProvider([[events.response()]]),
  })

  expect(await registry.listModels('stub', { region: 'test-region' })).toMatchObject({
    providerId: 'stub',
    defaultModelId: 'model-1',
    warnings: ['test-region'],
  })
  await expect(registry.listModels('no-catalog', {})).rejects.toThrow('does not expose a model catalog')
  await expect(registry.listModels('missing', {})).rejects.toThrow('is not registered')
})
