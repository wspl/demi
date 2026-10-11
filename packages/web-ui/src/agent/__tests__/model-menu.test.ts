import { expect, test } from 'bun:test'
import type { ModelInfo, ProviderInfo } from '../../transport/protocol'
import { modelMenuRows } from '../model-menu'

// A profile's model is chosen with the composer's own menu, the same items
// plus Parent's Model at its top (`product.md` § Settings): nothing the
// composer offers for a model is cut from the profile's menu.

const providers: ProviderInfo[] = [
  { id: 'anthropic', label: 'Anthropic', isAvailable: true },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
]
const opus: ModelInfo = {
  id: 'claude-opus',
  name: 'Claude Opus',
  contextWindow: 1_000_000,
  contextLimit: 500_000,
  acceptedExtensions: null,
  reasoning: { efforts: ['low', 'medium', 'high', 'max'], canDisable: true },
  serviceTiers: [{ id: 'fast', label: 'Fast', fast: true }],
}
const gpt: ModelInfo = { ...opus, id: 'gpt-5', name: 'GPT-5', contextWindow: 272_000, contextLimit: null }
const models = { anthropic: [opus], openai: [gpt] }

test('the profile menu offers the composer’s items for a model with Fast and context limits, plus Parent’s Model', () => {
  const settings = { providerId: 'anthropic', modelId: 'claude-opus', thinkingEffort: 'high', serviceTierId: 'fast' }
  const composer = modelMenuRows(providers, models, settings, false)
  const profile = modelMenuRows(providers, models, settings, true)

  expect(profile).toEqual({ ...composer, parent: { selected: false } })
  expect(composer.parent).toBeNull()
  expect(composer.fast).toEqual({ tier: opus.serviceTiers![0]!, on: true })
  expect(composer.reasoning?.value).toBe('High')
  expect(composer.reasoning?.options.map((option) => option.label)).toEqual(['Off', 'Low', 'Medium', 'High', 'Max'])
  expect(composer.context?.value).toBe('500K')
  expect(composer.context?.options.map((option) => [option.label, option.selected])).toEqual([
    ['1M', false],
    ['500K', true],
    ['300K', false],
    ['200K', false],
  ])
  expect(composer.model.value).toBe('Claude Opus')
  expect(composer.model.groups.map((group) => group.models.map((item) => [item.model.name, item.selected]))).toEqual([
    [['Claude Opus', true]],
    [['GPT-5', false]],
  ])
})

test('with Parent’s Model chosen, the profile menu marks it and no model', () => {
  const rows = modelMenuRows(providers, models, null, true)
  expect(rows.parent).toEqual({ selected: true })
  expect(rows.selected).toBeNull()
  expect(rows.model.value).toBe('')
  expect(rows.model.groups.flatMap((group) => group.models).some((item) => item.selected)).toBe(false)
})
