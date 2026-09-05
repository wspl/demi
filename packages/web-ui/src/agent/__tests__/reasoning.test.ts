import { test, expect } from 'bun:test'
import type { ModelInfo } from '../../transport/protocol'
import { buildReasoningState, reasoningOptionConfig, reasoningOptionIndex, reasoningOptionLabel } from '../reasoning'

const base: ModelInfo = {
  id: 'm',
  name: 'M',
  contextWindow: 200000,
  inputLimit: null,
  acceptedExtensions: [],
  reasoning: { efforts: ['low', 'medium', 'high'], defaultEffort: null, canDisable: true },
  serviceTiers: null,
}

test('offers Off when the model can disable thinking', () => {
  const state = buildReasoningState(base)!
  expect(state.canDisable).toBe(true)
  expect(state.options.map((o) => o.label)).toContain('Off')
  expect(state.options[0]!.config).toEqual({ type: 'disabled' })
})

test('omits Off when the model cannot disable thinking (e.g. Claude Code)', () => {
  const state = buildReasoningState({ ...base, reasoning: { ...base.reasoning!, canDisable: false } })!
  expect(state.canDisable).toBe(false)
  expect(state.options.map((o) => o.label)).not.toContain('Off')
  expect(state.options.every((o) => o.config.type === 'effort')).toBe(true)
  // the default selection is still a real effort, never "disabled"
  expect(state.defaultConfig).toEqual({ type: 'effort', effort: 'low', summary: null })
})

test('no reasoning state when the model has no efforts', () => {
  expect(buildReasoningState({ ...base, reasoning: null })).toBeNull()
})

test('option index maps disabled and effort configs', () => {
  const state = buildReasoningState(base)!
  expect(reasoningOptionIndex(state, { type: 'disabled' })).toBe(0)
  expect(reasoningOptionIndex(state, { type: 'effort', effort: 'medium', summary: null })).toBe(2)
  expect(reasoningOptionConfig(state, 0)).toEqual({ type: 'disabled' })
  expect(reasoningOptionConfig(state, 3)).toEqual({ type: 'effort', effort: 'high', summary: null })
  expect(reasoningOptionLabel(state, { type: 'disabled' })).toBe('Off')
  expect(reasoningOptionLabel(state, { type: 'effort', effort: 'high', summary: null })).toBe('High')
})
