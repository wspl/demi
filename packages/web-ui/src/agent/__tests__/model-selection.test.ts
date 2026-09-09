import { expect, test } from 'bun:test'
import type { ModelInfo, ProviderInfo } from '../../transport/protocol'
import { availableProviders, composerModel, resolveSelectedModel } from '../model-selection'

function model(id: string): ModelInfo {
  return {
    id,
    name: id.toUpperCase(),
    contextWindow: null,
    inputLimit: null,
    acceptedExtensions: [],
    reasoning: null,
    serviceTiers: null
  }
}

const providers: ProviderInfo[] = [
  { id: 'offline', label: 'Offline', isAvailable: false },
  { id: 'empty', label: 'Empty', isAvailable: true },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
  { id: 'anthropic', label: 'Anthropic', isAvailable: true },
]
const models: Record<string, ModelInfo[]> = {
  offline: [model('o1')],
  empty: [],
  openai: [model('gpt'), model('gpt-mini')],
  anthropic: [model('sonnet')],
}

test('usable providers are available and carry at least one model', () => {
  expect(availableProviders(providers, models).map((provider) => provider.id)).toEqual(
    ['openai', 'anthropic']
  )
})

test('the session choice wins when it is still usable', () => {
  expect(
    resolveSelectedModel(providers, models, 'anthropic', 'sonnet')?.model.name
  ).toBe(
    'SONNET'
  )
  expect(composerModel(providers, models, 'anthropic', 'sonnet').kind).toBe(
    'ready'
  )
})

test('an unavailable last choice stays put when other models exist', () => {
  const offline = composerModel(providers, models, 'offline', 'o1')
  expect(offline.kind).toBe('unavailable')
  expect(offline.label).toBe('O1')
  expect(resolveSelectedModel(providers, models, 'offline', 'o1')).toBeNull()

  const gone = composerModel(providers, models, 'anthropic', 'gone')
  expect(gone.kind).toBe('unavailable')
  expect(gone.label).toBe('gone')
  expect(resolveSelectedModel(providers, models, 'anthropic', 'gone')).toBeNull()
})

test('a session with no last choice falls through to the first usable model', () => {
  expect(resolveSelectedModel(providers, models, null, null)?.modelId).toBe('gpt')
  expect(composerModel(providers, models, null, null).kind).toBe('ready')
})

test('an empty catalog is configure-only', () => {
  expect(composerModel(providers, { openai: [] }, null, null).kind).toBe('none')
  expect(resolveSelectedModel(providers, { openai: [] }, null, null)).toBeNull()
  expect(composerModel(providers, { openai: [] }, 'offline', 'o1').kind).toBe(
    'none'
  )
})
