import { expect, test } from 'bun:test'
import type { ModelInfo, ProviderInfo } from '../../transport/protocol'
import { availableProviders, resolveSelectedModel } from '../model-selection'

function model(id: string): ModelInfo {
  return { id, name: id.toUpperCase(), contextWindow: null, inputLimit: null, acceptedExtensions: [], reasoning: null, serviceTiers: null }
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
  expect(availableProviders(providers, models).map((provider) => provider.id)).toEqual(['openai', 'anthropic'])
})

test('the session choice wins when the catalog has it', () => {
  expect(resolveSelectedModel(providers, models, 'anthropic', 'sonnet')?.model.name).toBe('SONNET')
})

test('an unknown choice falls back to the first model of the first usable provider', () => {
  expect(resolveSelectedModel(providers, models, 'anthropic', 'gone')?.modelId).toBe('gpt')
  expect(resolveSelectedModel(providers, models, null, null)?.modelId).toBe('gpt')
})

test('an empty catalog resolves nothing', () => {
  expect(resolveSelectedModel(providers, { openai: [] }, null, null)).toBeNull()
})
