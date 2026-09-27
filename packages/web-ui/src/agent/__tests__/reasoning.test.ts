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
    defaultEffort: null,
    canDisable: true
  },
  serviceTiers: null,
}

test('offers Off when the model can disable thinking', () => {
  const state = buildReasoningState(base)!
  expect(state.canDisable).toBe(true)
  expect(state.options.map((o) => o.label)).toContain('Off')
  expect(state.options[0]!.effort).toBe('disabled')
})

test('omits Off when the model cannot disable thinking (e.g. Claude Code)', () => {
  const state = buildReasoningState(
    {
      ...base,
      reasoning: {
        ...base.reasoning!,
        canDisable: false
      }
    }
  )!
  expect(state.canDisable).toBe(false)
  expect(state.options.map((o) => o.label)).not.toContain('Off')
  // the default selection is still a real effort, never "disabled"
  expect(state.defaultEffort).toBe('low')
})

test('no reasoning state when the model has no efforts', () => {
  expect(buildReasoningState({ ...base, reasoning: null })).toBeNull()
})

test('an effort shows its own option, and one the model does not offer its default', () => {
  const state = buildReasoningState(base)!
  expect(reasoningOptionIndex(state, 'disabled')).toBe(0)
  expect(reasoningOptionIndex(state, 'medium')).toBe(2)
  expect(reasoningOptionLabel(state, 'disabled')).toBe('Off')
  expect(reasoningOptionLabel(state, 'high')).toBe('High')
  expect(reasoningOptionLabel(state, 'xhigh')).toBe('Low')
})
