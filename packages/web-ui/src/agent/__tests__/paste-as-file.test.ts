import { expect, test } from 'bun:test'
import {
  PASTE_AS_FILE_MIN_CHARS,
  PASTE_AS_FILE_MIN_LINES,
  pastedTextFile,
  pastedTextIsLong,
} from '../message-input/attachments'

test('a paste is long by characters or by lines', () => {
  expect(pastedTextIsLong('hello')).toBe(false)
  expect(pastedTextIsLong('x'.repeat(PASTE_AS_FILE_MIN_CHARS - 1))).toBe(false)
  expect(pastedTextIsLong('x'.repeat(PASTE_AS_FILE_MIN_CHARS))).toBe(true)
  expect(pastedTextIsLong('line\n'.repeat(PASTE_AS_FILE_MIN_LINES - 2))).toBe(false)
  expect(pastedTextIsLong('line\n'.repeat(PASTE_AS_FILE_MIN_LINES))).toBe(true)
})

test('a pasted text file is plain text named pasted-text.txt', async () => {
  const file = pastedTextFile('body')
  expect(file.name).toBe('pasted-text.txt')
  expect(file.type.startsWith('text/plain')).toBe(true)
  expect(await file.text()).toBe('body')
})
