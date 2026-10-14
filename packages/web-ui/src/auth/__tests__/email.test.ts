import { expect, test } from 'bun:test'
import { isEmail } from '../email'

test('accepts a trimmed address and refuses an empty or bare name', () => {
  expect(isEmail('alex@example.com')).toBe(true)
  expect(isEmail('  alex@example.com  ')).toBe(true)
  expect(isEmail('Alex')).toBe(false)
  expect(isEmail('')).toBe(false)
  expect(isEmail('alex@example')).toBe(false)
})
