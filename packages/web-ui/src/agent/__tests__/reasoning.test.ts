import { test, expect } from 'bun:test'
import type { ModelInfo } from '../../transport/protocol'
import {
  buildReasoningState,
  reasoningOptionIndex,
  reasoningOptionLabel
} from '../reasoning'

const base: ModelInfo = {
  id: 'm',
  name: 'M',
  contextWindow: 200000,
  inputLimit: null,
  acceptedExtensions: [],
  reasoning: {
    efforts: ['low', 'medium', 'high'],
    unnamedEffort: null,
    canDisable: true
  },
  serviceTiers: null,
}

/** The same efforts on a model that cannot turn thinking off, for which the backend holds `medium` when none is named. */
const alwaysThinking: ModelInfo = {
  ...base,
  reasoning: { ...base.reasoning!, unnamedEffort: 'medium', canDisable: false },
}

test('a model that can turn thinking off offers its default and Off before its efforts, and no effort shows as the default', () => {
  const state = buildReasoningState(base)!
  expect(state.options.map((option) => [option.label, option.effort])).toEqual([
    ['Default', null],
    ['Off', 'disabled'],
    ['Low', 'low'],
    ['Medium', 'medium'],
    ['High', 'high'],
  ])
  // Its request then names no thinking setting, which Default says.
  expect(reasoningOptionLabel(state, null)).toBe('Default')
})

test('a model that cannot turn thinking off offers only its efforts, and no effort shows the one its requests carry', () => {
  const state = buildReasoningState(alwaysThinking)!
  expect(state.options.map((option) => option.label)).toEqual(['Low', 'Medium', 'High'])
  expect(reasoningOptionLabel(state, null)).toBe('Medium')
})

test('no reasoning state when the model has no efforts', () => {
  expect(buildReasoningState({ ...base, reasoning: null })).toBeNull()
})

test('an effort shows its own option, and one the model does not offer shows what none shows', () => {
  const state = buildReasoningState(base)!
  expect(reasoningOptionIndex(state, 'disabled')).toBe(1)
  expect(reasoningOptionIndex(state, 'medium')).toBe(3)
  expect(reasoningOptionLabel(state, 'high')).toBe('High')
  expect(reasoningOptionLabel(state, 'xhigh')).toBe('Default')
  expect(reasoningOptionLabel(buildReasoningState(alwaysThinking)!, 'xhigh')).toBe('Medium')
})
