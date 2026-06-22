import { test, expect } from 'bun:test'
import { buildReasoningState } from '../reasoning'
import type { ModelInfo } from '../../transport/protocol'

const base: ModelInfo = {
  id: 'm',
  name: 'M',
  contextWindow: 200000,
  inputLimit: null,
  acceptedExtensions: [],
  reasoning: { efforts: ['low', 'medium', 'high'], defaultEffort: null, canDisable: true },
}

test('offers "No reasoning" when the model can disable thinking', () => {
  const state = buildReasoningState(base)!
  expect(state.canDisable).toBe(true)
  expect(state.options.map((o) => o.label)).toContain('No reasoning')
  expect(state.options[0]!.config).toEqual({ type: 'disabled' })
})

test('omits "No reasoning" when the model cannot disable thinking (e.g. Claude Code)', () => {
  const state = buildReasoningState({ ...base, reasoning: { ...base.reasoning!, canDisable: false } })!
  expect(state.canDisable).toBe(false)
  expect(state.options.map((o) => o.label)).not.toContain('No reasoning')
  expect(state.options.every((o) => o.config.type === 'effort')).toBe(true)
  // the default selection is still a real effort, never "disabled"
  expect(state.defaultConfig).toEqual({ type: 'effort', effort: 'low', summary: null })
})

test('no reasoning state when the model has no efforts', () => {
  expect(buildReasoningState({ ...base, reasoning: null })).toBeNull()
})
