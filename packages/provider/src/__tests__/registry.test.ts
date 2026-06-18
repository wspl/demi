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
