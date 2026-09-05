import { expect, test } from 'bun:test'
import { composerHasLineBreak } from '../message-input/composer-multiline'

test('composer stays capsule until a line break exists', () => {
  expect(composerHasLineBreak(1, '')).toBe(false)
  expect(composerHasLineBreak(1, 'one line')).toBe(false)
  expect(composerHasLineBreak(2, 'one\ntwo')).toBe(true)
  expect(composerHasLineBreak(1, 'one\ntwo')).toBe(true)
})
