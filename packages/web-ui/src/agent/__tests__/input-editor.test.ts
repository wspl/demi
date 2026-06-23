import { expect, test } from 'bun:test'
import { shouldSubmitFromEditorKeydown } from '../message-input/useAgentInputEditor'

test('editor keydown submits only bare Enter', () => {
  expect(shouldSubmitFromEditorKeydown({ isComposing: false, key: 'Enter', shiftKey: false })).toBe(true)
  expect(shouldSubmitFromEditorKeydown({ isComposing: false, key: 'Enter', shiftKey: true })).toBe(false)
  expect(shouldSubmitFromEditorKeydown({ isComposing: true, key: 'Enter', shiftKey: false })).toBe(false)
  expect(shouldSubmitFromEditorKeydown({ isComposing: false, key: 'a', shiftKey: false })).toBe(false)
})
