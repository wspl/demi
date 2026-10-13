import { expect, test } from 'bun:test'
import { endpointProblem, vendorsInListOrder } from '../provider-defaults'
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

test('a compatible API takes an http or https address as its endpoint and nothing else', () => {
  expect(['https://proxy.example/v1', ' http://localhost:11434/v1 '].map(endpointProblem)).toEqual([null, null])
  expect(['', 'proxy.example/v1', 'ftp://proxy.example', 'not an address'].map(endpointProblem)).toEqual(
    Array(4).fill('Enter an http or https address.'),
  )
})
