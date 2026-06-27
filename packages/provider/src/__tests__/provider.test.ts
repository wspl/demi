import { expect, test } from 'bun:test'
import { applyModelPolicy, defineProvider, providerRuntime, type ProviderModelList, type ProviderSelection } from '../index'
import { StubProvider, events } from '../testing'

const selection: ProviderSelection = {
  providerId: 'stub',
  model: {
    providerId: 'stub',
    model: {
      id: 'model-1',
      name: 'Model 1',
      contextWindow: 100,
      inputLimit: null,
      thinking: [],
      acceptedExtensions: [],
    },
    thinking: null,
  },
}

test('defineProvider exposes public provider fields and hides runtime factory from serialization', async () => {
  const provider = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    state: () => ({ status: 'ready' }),
    createRuntime: () => new StubProvider([[events.text('ok'), events.response()]]),
  })

  expect(provider).toMatchObject({ id: 'stub', displayName: 'Stub' })
  expect(Object.keys(provider)).toEqual(['id', 'displayName', 'state'])
  expect(JSON.stringify(provider)).toBe('{"id":"stub","displayName":"Stub"}')

  const runtime = await providerRuntime(provider, selection)
  const output = []
  for await (const event of runtime.run({
    sessionId: 'test-session',
    turnId: 'test-turn',
    requestId: 'test-request',
    modelId: 'model-1',
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

test('applyModelPolicy remaps provider ids and applies include, exclude, and default selection', () => {
  const list: ProviderModelList = {
    providerId: 'source',
    defaultModelId: 'model-2',
    warnings: ['warning'],
    sourceFetchedAt: '2026-06-25T00:00:00.000Z',
    stale: false,
    models: [
      model('source', 'model-1'),
      model('source', 'model-2'),
      model('source', 'model-3'),
    ],
  }

  const filtered = applyModelPolicy(list, 'custom', {
    include: ['model-1', 'model-2', 'model-3'],
    exclude: ['model-2'],
    default: 'model-3',
  })

  expect(filtered.providerId).toBe('custom')
  expect(filtered.models.map((entry) => [entry.providerId, entry.id])).toEqual([
    ['custom', 'model-1'],
    ['custom', 'model-3'],
  ])
  expect(filtered.defaultModelId).toBe('model-3')
  expect(filtered.warnings).toEqual(['warning'])
})

function model(providerId: string, id: string): ProviderModelList['models'][number] {
  return {
    providerId,
    id,
    displayName: id,
    contextWindow: 100,
    outputLimit: null,
    supportsTools: null,
    supportsAttachments: null,
    supportsReasoning: null,
    supportedThinkingEfforts: null,
    defaultThinkingEffort: null,
    sourceFetchedAt: '2026-06-25T00:00:00.000Z',
    stale: false,
  }
}
