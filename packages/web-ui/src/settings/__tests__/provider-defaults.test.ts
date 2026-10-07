import { expect, test } from 'bun:test'
import { vendorsInListOrder } from '../provider-defaults'
import type { SettingsVendor } from '../types'

// Cost: pure function; under a millisecond.
function vendor(id: string): SettingsVendor {
  return { id, name: id, wireApi: 'openai-chat', baseUrl: null, logo: '' }
}

test('Add Provider lists OpenAI, Anthropic and Google first, then the catalog as it comes', () => {
  const catalog = ['deepseek', 'google', 'groq', 'anthropic', 'openai', 'mistral'].map(vendor)
  expect(vendorsInListOrder(catalog).map((entry) => entry.id)).toEqual([
    'openai',
    'anthropic',
    'google',
    'deepseek',
    'groq',
    'mistral',
  ])
})
