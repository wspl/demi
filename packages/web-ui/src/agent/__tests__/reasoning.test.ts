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
    canDisable: true
  },
  serviceTiers: null,
}

/** The same efforts on a model that cannot turn thinking off. */
const alwaysThinking: ModelInfo = {
  ...base,
  reasoning: { ...base.reasoning!, canDisable: false },
}

test('a model that can turn thinking off offers Off and its efforts, and no choice that leaves thinking to the vendor', () => {
  const state = buildReasoningState(base)!
  expect(state.options.map((option) => [option.label, option.effort])).toEqual([
    ['Off', 'disabled'],
    ['Low', 'low'],
    ['Medium', 'medium'],
    ['High', 'high'],
  ])
  // Settings that name no effort show the first, which the request carries.
  expect(reasoningOptionLabel(state, null)).toBe('Low')
})

test('a model that cannot turn thinking off offers only its efforts', () => {
  const state = buildReasoningState(alwaysThinking)!
  expect(state.options.map((option) => option.label)).toEqual(['Low', 'Medium', 'High'])
  expect(reasoningOptionLabel(state, null)).toBe('Low')
})

test('no reasoning state when the model has no efforts', () => {
  expect(buildReasoningState({ ...base, reasoning: null })).toBeNull()
  expect(buildReasoningState({ ...base, reasoning: { efforts: [], canDisable: true } })).toBeNull()
})

test('an effort shows its own option, and one the model does not offer shows the first effort', () => {
  const state = buildReasoningState(base)!
  expect(reasoningOptionIndex(state, 'disabled')).toBe(0)
  expect(reasoningOptionIndex(state, 'medium')).toBe(2)
  expect(reasoningOptionLabel(state, 'high')).toBe('High')
  expect(reasoningOptionLabel(state, 'xhigh')).toBe('Low')
  expect(reasoningOptionLabel(buildReasoningState(alwaysThinking)!, 'disabled')).toBe('Low')
})
