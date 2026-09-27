import { expect, test } from 'bun:test'
import type { ModelInfo, ProviderInfo } from '../../transport/protocol'
import {
  availableProviders,
  composerModel,
  initialModelSettings,
  modelSwitch,
  offeredSettings,
  resolveSelectedModel,
  type ModelSettingsChange,
} from '../model-selection'

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

test('new conversations keep independent complete choices including unavailable models', () => {
  const saved = {
    providerId: 'offline', modelId: 'o1',
    thinkingEffort: 'high', serviceTierId: 'priority',
  }
  const first = initialModelSettings(saved)
  expect(composerModel(providers, models, first.providerId, first.modelId).kind).toBe('unavailable')
  first.thinkingEffort = 'disabled'
  expect(initialModelSettings(saved)).toEqual(saved)
  expect(initialModelSettings()).toEqual({ providerId: '', modelId: '', thinkingEffort: null, serviceTierId: null })
})

/** A model that levels its thinking with `efforts`, and has `fast` as its Fast tier when given. */
function leveled(id: string, efforts: string[], options: { canDisable?: boolean; fast?: string } = {}): ModelInfo {
  return {
    ...model(id),
    reasoning: { efforts, defaultEffort: null, canDisable: options.canDisable ?? true },
    serviceTiers: options.fast ? [{ id: options.fast, label: 'Fast', fast: true }] : null,
  }
}

test('a switch keeps the effort and Fast the new model offers, and leaves the rest to its defaults', () => {
  const current = leveled('a', ['low', 'high'], { fast: 'priority' })
  const on = { thinkingEffort: 'high', serviceTierId: 'priority' }
  const off = { thinkingEffort: 'disabled', serviceTierId: null }
  const cases: [typeof on | typeof off, ModelInfo, ModelSettingsChange][] = [
    // The new model lists the effort and has a Fast tier of its own.
    [on, leveled('b', ['high'], { fast: 'flex' }), { thinkingEffort: 'high', serviceTierId: 'flex' }],
    // It lists neither.
    [on, leveled('b', ['medium']), {}],
    // Thinking stays off where it can be turned off.
    [off, leveled('b', ['low']), { thinkingEffort: 'disabled' }],
    [off, leveled('b', ['low'], { canDisable: false }), {}],
    // Fast that was off stays off.
    [off, leveled('b', ['low'], { fast: 'priority' }), { thinkingEffort: 'disabled' }],
  ]
  for (const [settings, next, kept] of cases) {
    expect(modelSwitch(settings, current, 'p', next)).toEqual({ model: { providerId: 'p', modelId: 'b' }, ...kept })
  }
})

test('a first send names only the parts its model still offers', () => {
  const settings = { providerId: 'p', modelId: 'a', thinkingEffort: 'xhigh', serviceTierId: 'priority' }
  expect(offeredSettings(settings, leveled('a', ['low'], { fast: 'priority' })))
    .toEqual({ ...settings, thinkingEffort: null })
  expect(offeredSettings({ ...settings, thinkingEffort: 'low' }, leveled('a', ['low'])))
    .toEqual({ ...settings, thinkingEffort: 'low', serviceTierId: null })
})

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
