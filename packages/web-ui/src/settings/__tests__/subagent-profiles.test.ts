import { expect, test } from 'bun:test'
import type { ModelInfo, ProviderInfo } from '../../transport/protocol'
import { profileMissing } from '../subagent-profiles'

const providers: ProviderInfo[] = [{ id: 'work', label: 'Work', isAvailable: true }]
const haiku: ModelInfo = {
  id: 'haiku',
  name: 'Haiku',
  contextWindow: 200_000,
  contextLimit: null,
  acceptedExtensions: null,
  reasoning: { efforts: ['low', 'high'], canDisable: true },
  serviceTiers: [{ id: 'priority', label: 'Priority', fast: true }],
}
const models = { work: [haiku] }

function settings(parts: Partial<{ providerId: string; modelId: string; thinkingEffort: string | null; serviceTierId: string | null }>) {
  return { providerId: 'work', modelId: 'haiku', thinkingEffort: 'low', serviceTierId: null, ...parts }
}

test('a profile is unavailable while its entry, model, effort or tier is gone, and names the part', () => {
  expect(profileMissing(null, providers, models)).toBeNull()
  expect(profileMissing(settings({}), providers, models)).toBeNull()
  expect(profileMissing(settings({ thinkingEffort: 'disabled', serviceTierId: 'priority' }), providers, models)).toBeNull()
  expect(profileMissing(settings({ providerId: 'gone' }), providers, models)).toBe(
    'Its provider is gone. Choose another model.',
  )
  expect(profileMissing(settings({ modelId: 'opus' }), providers, models)).toBe(
    'The model opus is gone. Choose another model.',
  )
  expect(profileMissing(settings({ thinkingEffort: 'minimal' }), providers, models)).toBe(
    'Haiku no longer offers the effort minimal. Choose another effort.',
  )
  expect(profileMissing(settings({ serviceTierId: 'flex' }), providers, models)).toBe(
    'Haiku no longer offers the tier flex. Choose another tier.',
  )
})
