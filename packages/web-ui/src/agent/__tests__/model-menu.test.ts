import { expect, test } from 'bun:test'
import type { ModelInfo, ProviderInfo } from '../../transport/protocol'
import { modelMenuRows } from '../model-menu'
import { profileContextChoice, profileContextOptions } from '../context-limit'

// A profile's model is chosen with the composer's own menu, the same items
// plus Parent's Model at its top (`web-application.md` § Web app scope): nothing the
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

  expect(profile).toEqual({ ...composer, standIn: { selected: false } })
  expect(composer.standIn).toBeNull()
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

// A profile's own Context (`subagents.md` § Profiles) offers the Context
// row's windows after Default, which names the window the user's limit
// gives; the full window is stored as its own size, never as null.
test('a profile’s Context offers Default with the user’s window, then the model’s windows, and nothing for a small window', () => {
  expect(profileContextOptions(opus)).toEqual([
    { label: 'Default (500K)', tokens: null },
    { label: '1M', tokens: 1_000_000 },
    { label: '500K', tokens: 500_000 },
    { label: '300K', tokens: 300_000 },
    { label: '200K', tokens: 200_000 },
  ])
  expect(profileContextOptions(gpt)).toEqual([])
  expect(profileContextChoice(opus, 300_000)).toBe(300_000)
  expect(profileContextChoice(opus, null)).toBeNull()
  // 500K stored on a model whose window grew to 800K, which offers no
  // 500K: the full window applies, as a session's does.
  expect(profileContextChoice({ ...opus, contextWindow: 800_000 }, 500_000)).toBe(800_000)
})

test('with Parent’s Model chosen, the profile menu marks it and no model', () => {
  const rows = modelMenuRows(providers, models, null, true)
  expect(rows.standIn).toEqual({ selected: true })
  expect(rows.selected).toBeNull()
  expect(rows.model.value).toBe('')
  expect(rows.model.groups.flatMap((group) => group.models).some((item) => item.selected)).toBe(false)
})

test('an incomplete entry\'s models are listed but cannot be chosen, with the backend\'s sentence as the reason', () => {
  const reason = 'Finish setting up this provider in Settings'
  const incomplete: ProviderInfo[] = [
    ...providers,
    { id: 'google', label: 'Google', isAvailable: false, setupReason: reason },
    // Unavailable for another reason (its key refused): its models stay out of the menu, as before.
    { id: 'broken', label: 'Broken', isAvailable: false },
  ]
  const rows = modelMenuRows(incomplete, { ...models, google: [{ ...gpt, id: 'gemini-flash', name: 'Gemini Flash' }], broken: [gpt] }, null, false)
  expect(rows.model.groups.map((group) => [group.provider.label, group.unchoosable])).toEqual([
    ['Anthropic', null],
    ['OpenAI', null],
    ['Google', reason],
  ])
  // An incomplete entry without a model has nothing to show dimmed.
  const empty = modelMenuRows(incomplete, { ...models, google: [] }, null, false)
  expect(empty.model.groups.map((group) => group.provider.label)).toEqual(['Anthropic', 'OpenAI'])
})
