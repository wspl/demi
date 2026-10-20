import { expect, test } from 'bun:test'

import { insertedText } from '../typed'

// Text put in without a key, as the Character Viewer or dictation does,
// reaches the device; a composition's draft waits for its end, and what
// deletes or replaces sends nothing. Planted defect this catches: such
// text dropped, as the view did before. Milliseconds.
test('the view sends text inserted without a key, and no composition draft', () => {
  expect(insertedText({ isComposing: false, inputType: 'insertText', data: 'ü' })).toBe('ü')
  expect(insertedText({ isComposing: true, inputType: 'insertCompositionText', data: '東' })).toBeUndefined()
  expect(insertedText({ isComposing: true, inputType: 'insertText', data: '東京' })).toBeUndefined()
  expect(insertedText({ isComposing: false, inputType: 'deleteContentBackward', data: null })).toBeUndefined()
})
