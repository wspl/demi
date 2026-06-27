import { expect, test } from 'bun:test'
import { editorHasContent, shouldSubmitFromEditorKeydown } from '../message-input/useAgentInputEditor'

test('editor keydown submits only bare Enter', () => {
  expect(shouldSubmitFromEditorKeydown({ isComposing: false, key: 'Enter', shiftKey: false })).toBe(true)
  expect(shouldSubmitFromEditorKeydown({ isComposing: false, key: 'Enter', shiftKey: true })).toBe(false)
  expect(shouldSubmitFromEditorKeydown({ isComposing: true, key: 'Enter', shiftKey: false })).toBe(false)
  expect(shouldSubmitFromEditorKeydown({ isComposing: false, key: 'a', shiftKey: false })).toBe(false)
})

test('editor content state follows editor emptiness', () => {
  expect(editorHasContent({ isEmpty: false })).toBe(true)
  expect(editorHasContent({ isEmpty: true })).toBe(false)
  expect(editorHasContent(null)).toBe(false)
})
